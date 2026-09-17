import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import { resumeExecution, startExecution, type FakeEffects, type RunResult } from "./host.ts";

const root = path.resolve(fileURLToPath(new URL("../../..", import.meta.url)));
export const wasmPath = path.join(root, "target/wasm32-unknown-unknown/release/tcc_wasm.wasm");
export const workerPath = fileURLToPath(new URL("./worker.ts", import.meta.url));

export type Workspace = {
  dir: string;
  dbPath: string;
  logPath: string;
  resultPath: string;
  configPath: string;
};

export type ConformanceOptions = {
  source: string;
  filename?: string;
  effects?: FakeEffects;
  eventPayload?: unknown;
  autoDeliverEvent?: boolean;
  expectedResult: unknown;
  expectedEffectLog?: string[];
  crashAts: string[];
  failCounts?: Record<string, number>;
  childArtifacts?: Record<string, string>;
  cancel?: boolean;
  executionId?: string;
};

export function compileArtifact(source: string, filename = "input.ts"): string {
  return canonicalStringify(compile(source, { filename }));
}

export function createWorkspace(): Workspace {
  const dir = mkdtempSync(path.join(tmpdir(), "tcc-conform-"));
  return {
    dir,
    dbPath: path.join(dir, "tcc.db"),
    logPath: path.join(dir, "effects.log"),
    resultPath: path.join(dir, "result.json"),
    configPath: path.join(dir, "config.json"),
  };
}

export function readEffectLog(logPath: string): string[] {
  try {
    const text = readFileSync(logPath, "utf8").trim();
    return text.length === 0 ? [] : text.split("\n");
  } catch {
    return [];
  }
}

export function runWorker(env: NodeJS.ProcessEnv): Promise<{ signal: string | null; status: number | null }> {
  return new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      ["--experimental-sqlite", "--experimental-strip-types", workerPath],
      {
        env: { ...process.env, ...env },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += String(chunk);
    });
    child.on("error", reject);
    child.on("exit", (status, signal) => {
      if (status && status !== 0 && signal === null) {
        reject(new Error(`worker exited ${status}: ${stderr}`));
        return;
      }
      resolve({ signal, status });
    });
  });
}

export async function runUninterrupted(options: {
  source: string;
  filename?: string;
  effects?: FakeEffects;
  eventPayload?: unknown;
  autoDeliverEvent?: boolean;
  failCounts?: Record<string, number>;
  childArtifacts?: Record<string, string>;
  cancel?: boolean;
  executionId?: string;
}): Promise<RunResult & { effectLog: string[] }> {
  const paths = createWorkspace();
  const artifactJson = compileArtifact(options.source, options.filename);
  const result = await startExecution({
    dbPath: paths.dbPath,
    wasmPath,
    artifactJson,
    effects: options.effects,
    eventPayload: options.eventPayload,
    autoDeliverEvent: options.autoDeliverEvent,
    failCounts: options.failCounts,
    childArtifacts: options.childArtifacts,
    cancel: options.cancel,
    executionId: options.executionId,
    effectLogPath: paths.logPath,
  });
  return { ...result, effectLog: readEffectLog(paths.logPath) };
}

export async function killAndResume(options: {
  source: string;
  filename?: string;
  crashAt: string;
  effects?: FakeEffects;
  eventPayload?: unknown;
  autoDeliverEvent?: boolean;
  failCounts?: Record<string, number>;
  resumeFailCounts?: Record<string, number>;
  childArtifacts?: Record<string, string>;
  cancel?: boolean;
  resumeCancel?: boolean;
  executionId?: string;
}): Promise<{ resumed: RunResult; effectLog: string[] }> {
  const paths = createWorkspace();
  const artifactJson = compileArtifact(options.source, options.filename);
  writeFileSync(
    paths.configPath,
    JSON.stringify({
      effects: options.effects ?? { generate: 42 },
      failCounts: options.failCounts,
      eventPayload: options.eventPayload,
      autoDeliverEvent: options.autoDeliverEvent,
      childArtifacts: options.childArtifacts,
      cancel: options.cancel,
    }),
  );
  const killed = await runWorker({
    TCC_MODE: "start",
    TCC_DB_PATH: paths.dbPath,
    TCC_WASM_PATH: wasmPath,
    TCC_ARTIFACT_JSON: artifactJson,
    TCC_CRASH_AT: options.crashAt,
    TCC_EFFECT_LOG_PATH: paths.logPath,
    TCC_CONFIG_PATH: paths.configPath,
    TCC_EXECUTION_ID: options.executionId ?? "first",
    TCC_AUTO_EVENT: options.autoDeliverEvent === false ? "0" : "1",
  });
  assert.equal(killed.signal, "SIGKILL");
  writeFileSync(
    paths.configPath,
    JSON.stringify({
      effects: options.effects ?? { generate: 42 },
      failCounts: options.resumeFailCounts,
      eventPayload: options.eventPayload,
      autoDeliverEvent: options.autoDeliverEvent,
      childArtifacts: options.childArtifacts,
      cancel: options.resumeCancel ?? options.cancel,
    }),
  );
  const resumed = await resumeExecution({
    dbPath: paths.dbPath,
    wasmPath,
    effects: options.effects,
    eventPayload: options.eventPayload,
    autoDeliverEvent: options.autoDeliverEvent,
    failCounts: options.resumeFailCounts,
    childArtifacts: options.childArtifacts,
    cancel: options.resumeCancel ?? options.cancel,
    executionId: options.executionId,
    effectLogPath: paths.logPath,
  });
  return { resumed, effectLog: readEffectLog(paths.logPath) };
}

/** Compile, run uninterrupted, then kill/resume at each durable boundary. */
export async function assertConformance(options: ConformanceOptions): Promise<void> {
  const clean = await runUninterrupted(options);
  assert.equal(clean.status, options.cancel ? "cancelled" : "completed");
  assert.deepEqual(clean.result, options.expectedResult);
  if (options.expectedEffectLog) {
    assert.deepEqual(clean.effectLog, options.expectedEffectLog);
  }
  for (const crashAt of options.crashAts) {
    const recovered = await killAndResume({ ...options, crashAt });
    assert.equal(recovered.resumed.status, options.cancel ? "cancelled" : "completed");
    assert.deepEqual(recovered.resumed.result, options.expectedResult);
    if (options.expectedEffectLog) {
      assert.deepEqual(recovered.effectLog, options.expectedEffectLog);
    }
  }
}
