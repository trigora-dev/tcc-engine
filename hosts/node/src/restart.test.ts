import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import { resumeExecution } from "./host.ts";
import { Store } from "./store.ts";

const SOURCE = `
import { effect, waitForEvent } from "@trigora/sdk";

export default async function run() {
  const result = await effect("generate", async () => {
    return generateSomething();
  });
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`;

const root = path.resolve(fileURLToPath(new URL("../../..", import.meta.url)));
const wasmPath = path.join(root, "target/wasm32-unknown-unknown/release/tcc_wasm.wasm");
const workerPath = fileURLToPath(new URL("./worker.ts", import.meta.url));

const completed = {
  t: "object",
  v: {
    approval: { t: "string", v: "ok" },
    result: { t: "number", v: 42 },
  },
};

function workspace(): { dir: string; dbPath: string; artifactPath: string; logPath: string; resultPath: string } {
  const dir = mkdtempSync(path.join(tmpdir(), "tcc-restart-"));
  return {
    dir,
    dbPath: path.join(dir, "tcc.db"),
    artifactPath: path.join(dir, "artifact.json"),
    logPath: path.join(dir, "effects.log"),
    resultPath: path.join(dir, "result.json"),
  };
}

function artifactJson(): string {
  return canonicalStringify(compile(SOURCE, { filename: "first.ts" }));
}

function runWorker(env: NodeJS.ProcessEnv): Promise<{ signal: string | null; status: number | null }> {
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

test("process death after a checkpoint still resumes", async () => {
  const paths = workspace();
  writeFileSync(paths.artifactPath, artifactJson());
  const killed = await runWorker({
    TCC_MODE: "start",
    TCC_DB_PATH: paths.dbPath,
    TCC_WASM_PATH: wasmPath,
    TCC_ARTIFACT_JSON: artifactJson(),
    TCC_CRASH_AT: "after_persist_checkpoint",
    TCC_AUTO_EVENT: "0",
    TCC_EFFECT_LOG_PATH: paths.logPath,
  });
  assert.equal(killed.signal, "SIGKILL");
  const store = new Store(paths.dbPath);
  const saved = store.getContinuation("first");
  store.close();
  assert.ok(saved);
  assert.ok(saved.revision >= 1);

  const resumed = await resumeExecution({
    dbPath: paths.dbPath,
    wasmPath,
    eventPayload: "ok",
    effectLogPath: paths.logPath,
  });
  assert.equal(resumed.status, "completed");
  assert.deepEqual(resumed.result, completed);
});

test("first example survives kill after wait commit", async () => {
  const paths = workspace();
  const killed = await runWorker({
    TCC_MODE: "start",
    TCC_DB_PATH: paths.dbPath,
    TCC_WASM_PATH: wasmPath,
    TCC_ARTIFACT_JSON: artifactJson(),
    TCC_CRASH_AT: "after_wait_checkpoint",
    TCC_EFFECT_LOG_PATH: paths.logPath,
  });
  assert.equal(killed.signal, "SIGKILL");

  const store = new Store(paths.dbPath);
  const wait = store.getWait("first:approved::4");
  const execution = store.getExecution("first");
  store.close();
  assert.equal(wait?.status, "pending");
  assert.equal(execution?.status, "suspended");

  const recovered = await runWorker({
    TCC_MODE: "resume",
    TCC_DB_PATH: paths.dbPath,
    TCC_WASM_PATH: wasmPath,
    TCC_EVENT_PAYLOAD: JSON.stringify("ok"),
    TCC_RESULT_PATH: paths.resultPath,
    TCC_EFFECT_LOG_PATH: paths.logPath,
    TCC_OWNER_TOKEN: "owner-1",
  });
  assert.equal(recovered.signal, null);
  assert.equal(recovered.status, 0);
  const result = JSON.parse(readFileSync(paths.resultPath, "utf8")) as {
    status: string;
    result: unknown;
  };
  assert.equal(result.status, "completed");
  assert.deepEqual(result.result, completed);
  const log = readFileSync(paths.logPath, "utf8").trim().split("\n");
  assert.equal(log.length, 1);
});

test("crash after journaled effect does not recall the provider", async () => {
  const paths = workspace();
  const killed = await runWorker({
    TCC_MODE: "start",
    TCC_DB_PATH: paths.dbPath,
    TCC_WASM_PATH: wasmPath,
    TCC_ARTIFACT_JSON: artifactJson(),
    TCC_CRASH_AT: "after_persist_effect",
    TCC_EFFECT_LOG_PATH: paths.logPath,
  });
  assert.equal(killed.signal, "SIGKILL");
  const resumed = await resumeExecution({
    dbPath: paths.dbPath,
    wasmPath,
    eventPayload: "ok",
    effectLogPath: paths.logPath,
  });
  assert.equal(resumed.status, "completed");
  const log = readFileSync(paths.logPath, "utf8").trim().split("\n");
  assert.equal(log.length, 1);
});

test("crash before journaled effect may recall the provider", async () => {
  const paths = workspace();
  const killed = await runWorker({
    TCC_MODE: "start",
    TCC_DB_PATH: paths.dbPath,
    TCC_WASM_PATH: wasmPath,
    TCC_ARTIFACT_JSON: artifactJson(),
    TCC_CRASH_AT: "after_effect_provider",
    TCC_EFFECT_LOG_PATH: paths.logPath,
  });
  assert.equal(killed.signal, "SIGKILL");
  const resumed = await resumeExecution({
    dbPath: paths.dbPath,
    wasmPath,
    eventPayload: "ok",
    effectLogPath: paths.logPath,
  });
  assert.equal(resumed.status, "completed");
  const log = readFileSync(paths.logPath, "utf8").trim().split("\n");
  assert.ok(log.length >= 1);
  assert.ok(log.length <= 2);
});

test("crash after register_wait still recovers the wait", async () => {
  const paths = workspace();
  const killed = await runWorker({
    TCC_MODE: "start",
    TCC_DB_PATH: paths.dbPath,
    TCC_WASM_PATH: wasmPath,
    TCC_ARTIFACT_JSON: artifactJson(),
    TCC_CRASH_AT: "after_register_wait",
  });
  assert.equal(killed.signal, "SIGKILL");
  const store = new Store(paths.dbPath);
  const wait = store.getWait("first:approved::4");
  store.close();
  assert.equal(wait?.event_name, "approved");

  const resumed = await resumeExecution({
    dbPath: paths.dbPath,
    wasmPath,
    eventPayload: "ok",
  });
  assert.equal(resumed.status, "completed");
  assert.deepEqual(resumed.result, completed);
});
