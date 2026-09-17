import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import { deliverDuplicateEvent, resumeExecution, startExecution } from "./host.ts";
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

const wasmPath = path.resolve(
  fileURLToPath(
    new URL("../../../target/wasm32-unknown-unknown/release/tcc_wasm.wasm", import.meta.url),
  ),
);

const completed = {
  t: "object",
  v: {
    approval: { t: "string", v: "ok" },
    result: { t: "number", v: 42 },
  },
};

function dbFile(): string {
  return path.join(mkdtempSync(path.join(tmpdir(), "tcc-host-")), "tcc.db");
}

function artifactJson(): string {
  return canonicalStringify(compile(SOURCE, { filename: "first.ts" }));
}

test("sqlite host completes the first example", async () => {
  const result = await startExecution({
    dbPath: dbFile(),
    wasmPath,
    artifactJson: artifactJson(),
  });
  assert.equal(result.status, "completed");
  assert.deepEqual(result.result, completed);
});

test("resume after wait uses the stored artifact hash", async () => {
  const dbPath = dbFile();
  const artifact = artifactJson();
  const first = await startExecution({
    dbPath,
    wasmPath,
    artifactJson: artifact,
    autoDeliverEvent: false,
  });
  assert.equal(first.status, "suspended");
  const store = new Store(dbPath);
  const execution = store.getExecution("first");
  const saved = store.getContinuation("first");
  store.close();
  assert.ok(execution);
  assert.ok(saved);
  assert.equal(JSON.parse(artifact).envelope.artifact_hash, execution.artifact_hash);
  const continuation = JSON.parse(saved.json) as { artifact_hash: string; status: string };
  assert.equal(continuation.artifact_hash, execution.artifact_hash);
  assert.equal(continuation.status, "suspended");

  const resumed = await resumeExecution({
    dbPath,
    wasmPath,
    eventPayload: "ok",
  });
  assert.equal(resumed.status, "completed");
  assert.deepEqual(resumed.result, completed);
});

test("completed effect is not invoked again on resume", async () => {
  const dbPath = dbFile();
  const logPath = path.join(path.dirname(dbPath), "effects.log");
  await startExecution({
    dbPath,
    wasmPath,
    artifactJson: artifactJson(),
    autoDeliverEvent: false,
    effectLogPath: logPath,
  });
  const store = new Store(dbPath);
  const effect = store.getEffect("first", "generate");
  store.close();
  assert.equal(effect?.status, "completed");

  const resumed = await resumeExecution({
    dbPath,
    wasmPath,
    effectLogPath: logPath,
    eventPayload: "ok",
  });
  assert.equal(resumed.status, "completed");
  const log = await (await import("node:fs/promises")).readFile(logPath, "utf8");
  assert.equal(log.trim(), "generate");
});

test("wait identity survives in sqlite", async () => {
  const dbPath = dbFile();
  await startExecution({
    dbPath,
    wasmPath,
    artifactJson: artifactJson(),
    autoDeliverEvent: false,
  });
  const store = new Store(dbPath);
  const wait = store.getWait("first:approved::4");
  store.close();
  assert.equal(wait?.status, "pending");
  assert.equal(wait?.event_name, "approved");
});

test("duplicate event after completion is a no-op", async () => {
  const dbPath = dbFile();
  const first = await startExecution({
    dbPath,
    wasmPath,
    artifactJson: artifactJson(),
  });
  assert.equal(first.status, "completed");
  deliverDuplicateEvent(dbPath, "first", "approved", "again");
  const again = await resumeExecution({ dbPath, wasmPath, eventPayload: "again" });
  assert.equal(again.status, "completed");
  assert.deepEqual(again.result, completed);
});

test("a second worker cannot take a live lease", async () => {
  const dbPath = dbFile();
  await startExecution({
    dbPath,
    wasmPath,
    artifactJson: artifactJson(),
    autoDeliverEvent: false,
    ownerToken: "alpha",
    leaseMs: 60_000,
  });
  await assert.rejects(
    () =>
      resumeExecution({
        dbPath,
        wasmPath,
        ownerToken: "beta",
        leaseMs: 60_000,
      }),
    /owned by another worker/,
  );
});
