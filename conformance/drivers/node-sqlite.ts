import { writeFileSync } from "node:fs";

import { canonicalStringify } from "../../frontends/typescript/src/canonical.ts";
import { compile } from "../../frontends/typescript/src/compile.ts";
import {
  createWorkspace,
  readEffectLog,
  runWorker,
  wasmPath,
} from "../../hosts/node/src/conformance.ts";
import { resumeExecution, startExecution } from "../../hosts/node/src/host.ts";
import { applyDelta } from "../../hosts/node/src/reconstruct.ts";
import { Store } from "../../hosts/node/src/store.ts";
import type {
  ConformanceHandle,
  ConformanceRunResult,
  CrashInput,
  HostConformanceDriver,
  ResumeInput,
  StartInput,
} from "../driver.ts";
import type { ContinuationDelta, ContinuationJson } from "../reconstruct.ts";

type WorkspaceState = {
  dbPath: string;
  logPath: string;
  configPath: string;
};

function encodeHandle(
  executionId: string,
  workspace: WorkspaceState,
): ConformanceHandle {
  return { executionId, workspace: JSON.stringify(workspace) };
}

function decodeHandle(handle: ConformanceHandle): WorkspaceState {
  return JSON.parse(handle.workspace) as WorkspaceState;
}

function workspaceFrom(paths: {
  dbPath: string;
  logPath: string;
  configPath: string;
}): WorkspaceState {
  return {
    dbPath: paths.dbPath,
    logPath: paths.logPath,
    configPath: paths.configPath,
  };
}

export function createNodeSqliteDriver(): HostConformanceDriver {
  return {
    language: "typescript",
    compile(source, filename) {
      return canonicalStringify(compile(source, { filename }));
    },
    applyDelta(base: ContinuationJson, delta: ContinuationDelta) {
      return applyDelta(base, delta);
    },
    async start(input: StartInput): Promise<ConformanceRunResult> {
      const paths = createWorkspace();
      const executionId = input.executionId ?? "first";
      const result = await startExecution({
        dbPath: paths.dbPath,
        wasmPath,
        artifactJson: input.artifactJson,
        executionId,
        effects: input.effects,
        eventPayload: input.eventPayload,
        autoDeliverEvent: input.autoDeliverEvent,
        completionOrder: input.completionOrder,
        childArtifacts: input.childArtifacts,
        cancel: input.cancel,
        effectLogPath: paths.logPath,
      });
      return {
        status: result.status,
        result: result.result,
        handle: encodeHandle(executionId, workspaceFrom(paths)),
      };
    },
    async crashAt(input: CrashInput): Promise<ConformanceHandle> {
      const paths = createWorkspace();
      const executionId = input.executionId ?? "first";
      writeFileSync(
        paths.configPath,
        JSON.stringify({
          effects: input.effects ?? { generate: 42 },
          eventPayload: input.eventPayload,
          autoDeliverEvent: input.autoDeliverEvent,
          completionOrder: input.completionOrder,
          childArtifacts: input.childArtifacts,
          cancel: input.cancel,
        }),
      );
      const killed = await runWorker({
        TCC_MODE: "start",
        TCC_DB_PATH: paths.dbPath,
        TCC_WASM_PATH: wasmPath,
        TCC_ARTIFACT_JSON: input.artifactJson,
        TCC_CRASH_AT: input.crashAt,
        TCC_EFFECT_LOG_PATH: paths.logPath,
        TCC_CONFIG_PATH: paths.configPath,
        TCC_EXECUTION_ID: executionId,
        TCC_AUTO_EVENT: input.autoDeliverEvent === false ? "0" : "1",
      });
      if (killed.signal !== "SIGKILL") {
        throw new Error(
          `expected SIGKILL, got ${killed.signal ?? killed.status}`,
        );
      }
      return encodeHandle(executionId, workspaceFrom(paths));
    },
    async resume(input: ResumeInput): Promise<ConformanceRunResult> {
      const workspace = decodeHandle(input.handle);
      const result = await resumeExecution({
        dbPath: workspace.dbPath,
        wasmPath,
        artifactJson: input.artifactJson,
        executionId: input.handle.executionId,
        effects: input.effects,
        eventPayload: input.eventPayload,
        autoDeliverEvent: input.autoDeliverEvent,
        completionOrder: input.completionOrder,
        childArtifacts: input.childArtifacts,
        cancel: input.cancel,
        effectLogPath: workspace.logPath,
      });
      return {
        status: result.status,
        result: result.result,
        handle: input.handle,
      };
    },
    async readContinuation(
      handle: ConformanceHandle,
    ): Promise<ContinuationJson> {
      const workspace = decodeHandle(handle);
      const store = new Store(workspace.dbPath);
      try {
        const saved = store.getContinuation(handle.executionId);
        if (!saved) {
          throw new Error(`missing continuation for ${handle.executionId}`);
        }
        return JSON.parse(saved.json) as ContinuationJson;
      } finally {
        store.close();
      }
    },
    async effectLog(handle: ConformanceHandle): Promise<string[]> {
      return readEffectLog(decodeHandle(handle).logPath);
    },
  };
}

export const createDriver = createNodeSqliteDriver;
export default createNodeSqliteDriver;
