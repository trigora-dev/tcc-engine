import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../frontends/typescript/src/canonical.ts";
import { compile } from "../../frontends/typescript/src/compile.ts";
import type {
  ConformanceHandle,
  ConformanceRunResult,
  CrashInput,
  HostConformanceDriver,
  ResumeInput,
  StartInput,
} from "../driver.ts";
import { applyDelta, type ContinuationDelta, type ContinuationJson } from "../reconstruct.ts";

const root = path.resolve(fileURLToPath(new URL("../..", import.meta.url)));
const hostBin =
  process.env.TCC_HOST_SQLITE ??
  path.join(root, "target/debug/tcc-host-sqlite");

type WorkspaceState = {
  dbPath: string;
  logPath: string;
  configPath: string;
};

function createWorkspace(): WorkspaceState {
  const dir = mkdtempSync(path.join(tmpdir(), "tcc-rust-sqlite-"));
  return {
    dbPath: path.join(dir, "host.db"),
    logPath: path.join(dir, "effects.log"),
    configPath: path.join(dir, "config.json"),
  };
}

function encodeHandle(
  executionId: string,
  workspace: WorkspaceState,
): ConformanceHandle {
  return { executionId, workspace: JSON.stringify(workspace) };
}

function decodeHandle(handle: ConformanceHandle): WorkspaceState {
  return JSON.parse(handle.workspace) as WorkspaceState;
}

function writeConfig(configPath: string, input: StartInput | ResumeInput): void {
  writeFileSync(
    configPath,
    JSON.stringify({
      effects: input.effects,
      eventPayload: input.eventPayload,
      autoDeliverEvent: input.autoDeliverEvent,
      completionOrder: input.completionOrder,
      childArtifacts: input.childArtifacts,
      cancel: input.cancel,
    }),
  );
}

type SpawnResult = {
  status: number | null;
  signal: NodeJS.Signals | null;
  stdout: string;
  stderr: string;
};

function runHost(env: NodeJS.ProcessEnv): Promise<SpawnResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(hostBin, [], {
      env: { ...process.env, ...env },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += String(chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderr += String(chunk);
    });
    child.on("error", reject);
    child.on("exit", (status, signal) => {
      resolve({ status, signal, stdout, stderr });
    });
  });
}

function parseRun(spawned: SpawnResult): { status: string; result: unknown } {
  if (spawned.signal) {
    throw new Error(`host died with ${spawned.signal}: ${spawned.stderr}`);
  }
  if (spawned.status !== 0) {
    throw new Error(spawned.stderr || spawned.stdout || `host exited ${spawned.status}`);
  }
  const line = spawned.stdout.trim().split("\n").at(-1);
  if (!line) {
    throw new Error(`host produced no result: ${spawned.stderr}`);
  }
  return JSON.parse(line) as { status: string; result: unknown };
}

function hostEnv(
  mode: string,
  workspace: WorkspaceState,
  executionId: string,
  extra: NodeJS.ProcessEnv,
): NodeJS.ProcessEnv {
  return {
    TCC_MODE: mode,
    TCC_DB_PATH: workspace.dbPath,
    TCC_EXECUTION_ID: executionId,
    TCC_EFFECT_LOG_PATH: workspace.logPath,
    TCC_CONFIG_PATH: workspace.configPath,
    ...extra,
  };
}

export function createRustSqliteDriver(): HostConformanceDriver {
  return {
    language: "typescript",
    compile(source, filename) {
      return canonicalStringify(compile(source, { filename }));
    },
    applyDelta(base: ContinuationJson, delta: ContinuationDelta) {
      return applyDelta(base, delta);
    },
    async start(input: StartInput): Promise<ConformanceRunResult> {
      const workspace = createWorkspace();
      const executionId = input.executionId ?? "first";
      writeConfig(workspace.configPath, input);
      const spawned = await runHost(
        hostEnv("start", workspace, executionId, {
          TCC_ARTIFACT_JSON: input.artifactJson,
          TCC_AUTO_EVENT: input.autoDeliverEvent === false ? "0" : "1",
        }),
      );
      const result = parseRun(spawned);
      return {
        status: result.status,
        result: result.result,
        handle: encodeHandle(executionId, workspace),
      };
    },
    async crashAt(input: CrashInput): Promise<ConformanceHandle> {
      const workspace = createWorkspace();
      const executionId = input.executionId ?? "first";
      writeConfig(workspace.configPath, input);
      const spawned = await runHost(
        hostEnv("start", workspace, executionId, {
          TCC_ARTIFACT_JSON: input.artifactJson,
          TCC_CRASH_AT: input.crashAt,
          TCC_AUTO_EVENT: input.autoDeliverEvent === false ? "0" : "1",
        }),
      );
      if (spawned.signal !== "SIGKILL") {
        throw new Error(
          `expected SIGKILL, got ${spawned.signal ?? spawned.status}: ${spawned.stderr}`,
        );
      }
      return encodeHandle(executionId, workspace);
    },
    async resume(input: ResumeInput): Promise<ConformanceRunResult> {
      const workspace = decodeHandle(input.handle);
      writeConfig(workspace.configPath, input);
      const extra: NodeJS.ProcessEnv = {
        TCC_AUTO_EVENT: input.autoDeliverEvent === false ? "0" : "1",
      };
      if (input.artifactJson) {
        extra.TCC_ARTIFACT_JSON = input.artifactJson;
      }
      const spawned = await runHost(
        hostEnv("resume", workspace, input.handle.executionId, extra),
      );
      const result = parseRun(spawned);
      return {
        status: result.status,
        result: result.result,
        handle: input.handle,
      };
    },
    async readContinuation(handle: ConformanceHandle): Promise<ContinuationJson> {
      const workspace = decodeHandle(handle);
      const spawned = await runHost(
        hostEnv("read", workspace, handle.executionId, {}),
      );
      if (spawned.status !== 0) {
        throw new Error(spawned.stderr || `read exited ${spawned.status}`);
      }
      return JSON.parse(spawned.stdout) as ContinuationJson;
    },
    async effectLog(handle: ConformanceHandle): Promise<string[]> {
      const workspace = decodeHandle(handle);
      try {
        const text = readFileSync(workspace.logPath, "utf8").trim();
        return text.length === 0 ? [] : text.split("\n");
      } catch {
        return [];
      }
    },
  };
}

export const createDriver = createRustSqliteDriver;
export default createRustSqliteDriver;
