import assert from "node:assert/strict";
import { mkdtempSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import {
  canonicalEffectInput,
  deliverDuplicateEvent,
  effectInputJson,
  resumeExecution,
  startExecution,
} from "./host.ts";
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

const INPUT = `
export default async function run(input) {
  return input;
}
`;

const TAIL = `
export default async function run(a, b) {
  return b;
}
`;

const NO_PARAM = `
export default async function run() {
  return 1;
}
`;

test("start pads a short vector and ignores extras", async () => {
  const tail = canonicalStringify(compile(TAIL, { filename: "tail.ts" }));
  const missing = await startExecution({
    dbPath: dbFile(),
    wasmPath,
    artifactJson: tail,
    args: ["hi"],
  });
  assert.equal(missing.status, "completed");
  assert.deepEqual(missing.result, { t: "undefined" });

  const explicit = await startExecution({
    dbPath: dbFile(),
    wasmPath,
    artifactJson: canonicalStringify(compile(INPUT, { filename: "input.ts" })),
    args: [null],
  });
  assert.deepEqual(explicit.result, { t: "null" });

  const ignored = await startExecution({
    dbPath: dbFile(),
    wasmPath,
    artifactJson: canonicalStringify(compile(NO_PARAM, { filename: "none.ts" })),
    args: [null],
  });
  assert.equal(ignored.status, "completed");
  assert.deepEqual(ignored.result, { t: "number", v: 1 });
});

test("first committed child args stay authoritative", () => {
  const store = new Store(dbFile());
  store.putArtifact("h", "{}");
  const base = {
    parentExecutionId: "parent",
    flowName: "analyze",
    artifactHash: "h",
    ownerToken: "o",
    leaseUntil: 1,
  };
  const first = '[{"t":"string","v":"first"}]';
  store.enqueueCreateChild({
    ...base,
    invokeId: "same",
    childExecutionId: "child",
    argsJson: first,
  });
  store.flushIfUngrouped();
  store.enqueueCreateChild({
    ...base,
    invokeId: "same",
    childExecutionId: "child",
    argsJson: first,
  });
  store.flushIfUngrouped();
  assert.equal(store.getChild("same")?.args_json, first);
  assert.throws(() => {
    store.enqueueCreateChild({
      ...base,
      invokeId: "same",
      childExecutionId: "child",
      argsJson: '[{"t":"string","v":"second"}]',
    });
    store.flushIfUngrouped();
  }, /argument vector mismatch/);
  assert.equal(store.getChild("same")?.args_json, first);
  store.enqueueCreateChild({
    ...base,
    invokeId: "absent",
    childExecutionId: "child-absent",
    argsJson: "[]",
  });
  store.flushIfUngrouped();
  assert.equal(store.getChild("absent")?.args_json, null);
  store.enqueueCreateChild({
    ...base,
    invokeId: "explicit-null",
    childExecutionId: "child-null",
    argsJson: '[{"t":"null"}]',
  });
  store.flushIfUngrouped();
  assert.equal(store.getChild("explicit-null")?.args_json, '[{"t":"null"}]');
  store.close();
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

test("effect input comparison is canonical", () => {
  const unsorted = {
    v: { z: { t: "number", v: 1 }, a: { t: "string", v: "q" } },
    t: "object",
  };
  const sorted = {
    t: "object",
    v: { a: { t: "string", v: "q" }, z: { t: "number", v: 1 } },
  };
  assert.equal(canonicalEffectInput(unsorted), canonicalEffectInput(sorted));
  assert.equal(effectInputJson(JSON.stringify(unsorted)), canonicalEffectInput(sorted));
  assert.throws(() => effectInputJson(null), /effect input_json is required/);
  assert.throws(() => canonicalEffectInput(undefined), /unsupported effect input/);
});

test("a fresh store requires effect input and recreates a nullable column", () => {
  const fresh = new Store(dbFile());
  const freshColumns = fresh.db.prepare("PRAGMA table_info(effects)").all() as {
    name: string;
    notnull: number;
  }[];
  fresh.close();
  assert.equal(freshColumns.find((column) => column.name === "input_json")?.notnull, 1);

  const dbPath = dbFile();
  const legacy = new DatabaseSync(dbPath);
  legacy.exec(`
    CREATE TABLE effects (
      execution_id TEXT NOT NULL,
      key TEXT NOT NULL,
      idempotency_key TEXT NOT NULL,
      status TEXT NOT NULL,
      result_json TEXT,
      input_json TEXT,
      PRIMARY KEY (execution_id, key)
    );
  `);
  legacy
    .prepare(
      `INSERT INTO effects(execution_id, key, idempotency_key, status, input_json)
       VALUES ('old', 'generate', 'old:generate', 'completed', NULL)`,
    )
    .run();
  legacy.close();
  const rebuilt = new Store(dbPath);
  const columns = rebuilt.db.prepare("PRAGMA table_info(effects)").all() as {
    name: string;
    notnull: number;
  }[];
  assert.equal(columns.find((column) => column.name === "input_json")?.notnull, 1);
  assert.equal(rebuilt.getEffect("old", "generate"), undefined);
  rebuilt.close();

  const missingPath = dbFile();
  const missing = new DatabaseSync(missingPath);
  missing.exec(`
    CREATE TABLE effects (
      execution_id TEXT NOT NULL,
      key TEXT NOT NULL,
      idempotency_key TEXT NOT NULL,
      status TEXT NOT NULL,
      result_json TEXT,
      PRIMARY KEY (execution_id, key)
    );
  `);
  missing.close();
  const added = new Store(missingPath);
  const addedColumns = added.db.prepare("PRAGMA table_info(effects)").all() as { name: string; notnull: number }[];
  assert.equal(addedColumns.find((column) => column.name === "input_json")?.notnull, 1);
  added.close();
});

test("a reordered empty effect input is a journal hit", async () => {
  const dbPath = dbFile();
  const store = new Store(dbPath);
  store.close();
  const db = new DatabaseSync(dbPath);
  db.prepare(
    `INSERT INTO effects(execution_id, key, idempotency_key, status, result_json, input_json)
     VALUES ('first', 'generate', 'first:generate', 'completed', ?, ?)`,
  ).run(JSON.stringify({ t: "number", v: 7 }), JSON.stringify({ v: {}, t: "object" }));
  db.close();
  const logPath = path.join(path.dirname(dbPath), "effects.log");
  const result = await startExecution({
    dbPath,
    wasmPath,
    artifactJson: artifactJson(),
    autoDeliverEvent: false,
    effectLogPath: logPath,
  });
  assert.equal(result.status, "suspended");
  assert.equal(existsSync(logPath), false);
});

test("a different effect input is an invariant error", async () => {
  const dbPath = dbFile();
  const store = new Store(dbPath);
  store.completeEffect(
    "first",
    "generate",
    "first:generate",
    JSON.stringify({ t: "number", v: 7 }),
    canonicalEffectInput({
      t: "object",
      v: { query: { t: "string", v: "hello" } },
    }),
  );
  store.close();
  await assert.rejects(
    () =>
      startExecution({
        dbPath,
        wasmPath,
        artifactJson: artifactJson(),
        autoDeliverEvent: false,
      }),
    /input mismatch/,
  );
});
