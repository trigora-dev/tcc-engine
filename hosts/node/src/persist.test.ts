import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import { killAndResume } from "./conformance.ts";
import { ensureEngineLoaded, resumeExecution, runBatchOnStore, runOnStore, startExecution } from "./host.ts";
import {
  applyDelta,
  MATERIALIZE_EVERY,
  reconstructContinuation,
  Store,
  type ContinuationJson,
  type HostEvent,
} from "./store.ts";

const wasmPath = path.resolve(
  fileURLToPath(
    new URL("../../../target/wasm32-unknown-unknown/release/tcc_wasm.wasm", import.meta.url),
  ),
);

const fixturesDir = path.resolve(
  fileURLToPath(new URL("../../../spec/fixtures/persist", import.meta.url)),
);

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

const completed = {
  t: "object",
  v: {
    approval: { t: "string", v: "ok" },
    result: { t: "number", v: 42 },
  },
};

function dbFile(): string {
  return path.join(mkdtempSync(path.join(tmpdir(), "tcc-persist-")), "tcc.db");
}

function artifactJson(): string {
  return canonicalStringify(compile(SOURCE, { filename: "first.ts" }));
}

function dummyContinuation(executionId: string, revision: number, extra: Record<string, unknown> = {}): string {
  return JSON.stringify({
    artifact_hash: "hash",
    engine_format_version: 1,
    execution_id: executionId,
    frames: [{ func_id: 0, locals: [{ t: "undefined" }], pc: revision }],
    language_semantics_version: "ts.subset.v1",
    pending: null,
    result: null,
    revision,
    stack: [],
    status: "runnable",
    try_stack: [],
    ...extra,
  });
}

test("batch checkpoints commit before confirmation across multiple rounds", async () => {
  await ensureEngineLoaded(wasmPath);
  const store = new Store(dbFile(), "optimized");
  const artifact = artifactJson();
  try {
    const inputs = ["batch-a", "batch-b"].map((executionId) => ({
      dbPath: "unused", artifactJson: artifact, executionId, effects: { generate: 42 },
    }));
    let rounds = 0;
    const results = runBatchOnStore(store, inputs, () => { rounds += 1; });
    assert.equal(rounds, 4);
    assert.deepEqual(results.map((result) => result.status), ["completed", "completed"]);
    assert.equal(store.metrics().commitCount, 4);
    assert.ok(store.metrics().batchOccupancySamples.every((count) => count === 2));
    for (const input of inputs) assert.equal(store.getExecution(input.executionId)?.revision, 4);
    store.beginGroup();
    assert.throws(() => runOnStore(store, inputs[0]!), /open group/);
    store.abortGroup();
  } finally {
    store.close();
  }
});

test("failed batch confirms none of its pending revisions", async () => {
  await ensureEngineLoaded(wasmPath);
  const dbPath = dbFile();
  const store = new Store(dbPath, "optimized");
  const artifact = artifactJson();
  try {
    let rounds = 0;
    assert.throws(() => runBatchOnStore(store, ["fail-a", "fail-b"].map((executionId) => ({
      dbPath, artifactJson: artifact, executionId, effects: { generate: 42 },
    })), () => {
      if (++rounds === 2) store.db.exec(`CREATE TEMP TRIGGER fail_checkpoint BEFORE INSERT ON persist_wal
        WHEN NEW.revision = 2 BEGIN SELECT RAISE(ABORT, 'injected commit failure'); END`);
    }), /injected commit failure/);
    for (const executionId of ["fail-a", "fail-b"]) {
      assert.equal(store.getExecution(executionId)?.revision, 1);
      assert.equal(store.getContinuation(executionId)?.revision, 1);
    }
  } finally {
    store.close();
  }
  for (const executionId of ["fail-a", "fail-b"]) {
    const result = await resumeExecution({ dbPath, executionId, artifactJson: artifact, effects: { generate: 42 }, persist: "optimized" });
    assert.equal(result.status, "completed");
  }
});

test("SIGKILL before batch commit restores the previous confirmed revision", async () => {
  const dbPath = dbFile();
  const artifact = artifactJson();
  const hostUrl = new URL("./host.ts", import.meta.url).href;
  const storeUrl = new URL("./store.ts", import.meta.url).href;
  const script = `import { ensureEngineLoaded, runBatchOnStore } from ${JSON.stringify(hostUrl)};
import { Store } from ${JSON.stringify(storeUrl)};
await ensureEngineLoaded(process.env.TCC_WASM);
const store = new Store(process.env.TCC_DB, "optimized");
let rounds = 0;
runBatchOnStore(store, ["crash-a", "crash-b"].map((executionId) => ({
  dbPath: process.env.TCC_DB, artifactJson: process.env.TCC_ARTIFACT, executionId, effects: { generate: 42 },
})), () => { if (++rounds === 2) process.kill(process.pid, "SIGKILL"); });`;
  const child = spawnSync(process.execPath, ["--experimental-sqlite", "--experimental-strip-types", "--input-type=module", "-e", script], {
    env: { ...process.env, TCC_DB: dbPath, TCC_WASM: wasmPath, TCC_ARTIFACT: artifact },
  });
  assert.equal(child.signal, "SIGKILL", child.stderr.toString());
  const store = new Store(dbPath, "optimized");
  try {
    for (const executionId of ["crash-a", "crash-b"]) assert.equal(store.getContinuation(executionId)?.revision, 1);
  } finally {
    store.close();
  }
  for (const executionId of ["crash-a", "crash-b"]) {
    const result = await resumeExecution({ dbPath, executionId, artifactJson: artifact, effects: { generate: 42 }, persist: "optimized" });
    assert.equal(result.status, "completed");
  }
});

test("reconstruct goldens match applyDelta", () => {
  const files = readdirSync(fixturesDir).filter((name) => name.endsWith(".json"));
  assert.ok(files.length >= 4);
  for (const name of files) {
    const fixture = JSON.parse(readFileSync(path.join(fixturesDir, name), "utf8")) as {
      base: ContinuationJson;
      delta: Parameters<typeof applyDelta>[1];
      expected: ContinuationJson;
    };
    const applied = applyDelta(fixture.base, fixture.delta);
    applied.revision = fixture.expected.revision;
    assert.deepEqual(applied, fixture.expected, name);
    assert.deepEqual(
      reconstructContinuation(fixture.base, [fixture.delta], fixture.expected.revision),
      fixture.expected,
      name,
    );
  }
});

test("naive and optimized hosts complete the first example with the same result", async () => {
  const artifact = artifactJson();
  const naive = await startExecution({
    dbPath: dbFile(),
    wasmPath,
    artifactJson: artifact,
    persist: "naive",
  });
  const optimized = await startExecution({
    dbPath: dbFile(),
    wasmPath,
    artifactJson: artifact,
    persist: "optimized",
  });
  assert.equal(naive.status, "completed");
  assert.equal(optimized.status, "completed");
  assert.deepEqual(naive.result, completed);
  assert.deepEqual(optimized.result, naive.result);
  assert.equal(optimized.revision, naive.revision);
});

test("optimized recover uses last snapshot plus a bounded suffix", async () => {
  const dbPath = dbFile();
  const artifact = artifactJson();
  const first = await startExecution({
    dbPath,
    wasmPath,
    artifactJson: artifact,
    autoDeliverEvent: false,
    persist: "optimized",
  });
  assert.equal(first.status, "suspended");
  const store = new Store(dbPath, "optimized");
  const plan = store.recoverPlan("first");
  const records = store.walRecords("first");
  store.close();
  assert.ok(plan);
  assert.ok(plan.suffixLength <= MATERIALIZE_EVERY);
  assert.ok(records.some((row) => row.kind === "snapshot"));
  assert.ok(records.some((row) => row.kind === "delta"));
});

test("materialization bound keeps WAL suffix at most N", () => {
  const dbPath = dbFile();
  const store = new Store(dbPath, "optimized");
  store.putArtifact("hash", "{}");
  store.createExecution("bound", "hash", "owner-1", Date.now() + 60_000);
  for (let revision = 1; revision <= MATERIALIZE_EVERY + 2; revision++) {
    const json = dummyContinuation("bound", revision);
    store.commitCheckpoint("bound", revision, json, "runnable", "owner-1", {
      kind: revision === 1 ? "snapshot" : "delta",
      materialize: revision === 1,
      deltaJson: JSON.stringify({
        frames: [{ index: 0, pc: revision }],
      }),
    });
  }
  const head = store.execHead("bound");
  const plan = store.recoverPlan("bound");
  const records = store.walRecords("bound");
  const saved = store.getContinuation("bound");
  store.close();
  assert.ok(head);
  assert.ok(plan);
  assert.ok(plan.suffixLength <= MATERIALIZE_EVERY);
  assert.equal(head.revision - head.snapshot_revision, plan.suffixLength);
  assert.ok(records.filter((row) => row.kind === "snapshot").length >= 2);
  assert.ok(saved);
  assert.equal(JSON.parse(saved.json).revision, MATERIALIZE_EVERY + 2);
});

test("recovery of one execution does not scan foreign WAL rows", () => {
  const dbPath = dbFile();
  const store = new Store(dbPath, "optimized");
  store.putArtifact("hash", "{}");
  store.beginGroup();
  for (let i = 0; i < 200; i++) {
    const id = `foreign-${i}`;
    store.createExecution(id, "hash", "owner-1", Date.now() + 60_000);
    store.commitCheckpoint(id, 1, dummyContinuation(id, 1), "runnable", "owner-1", {
      kind: "snapshot",
      materialize: true,
    });
  }
  store.createExecution("target", "hash", "owner-1", Date.now() + 60_000);
  store.commitCheckpoint("target", 1, dummyContinuation("target", 1), "runnable", "owner-1", {
    kind: "snapshot",
    materialize: true,
  });
  store.commitCheckpoint(
    "target",
    2,
    dummyContinuation("target", 2),
    "runnable",
    "owner-1",
    {
      kind: "delta",
      materialize: false,
      deltaJson: JSON.stringify({ frames: [{ index: 0, pc: 2 }] }),
    },
  );
  store.endGroup();
  const plan = store.recoverPlan("target");
  const saved = store.getContinuation("target");
  const own = store.walRecords("target").length;
  const total = store.walRowCount();
  store.close();
  assert.ok(plan);
  assert.equal(plan.suffixLength, 1);
  assert.equal(own, 2);
  assert.ok(total >= 202);
  assert.ok(plan.usedIndex, plan.detail);
  assert.ok(saved);
  assert.equal(JSON.parse(saved.json).frames[0].pc, 2);
});

test("optimized resume after wait matches naive resume", async () => {
  const artifact = artifactJson();
  async function run(persist: "naive" | "optimized") {
    const dbPath = dbFile();
    const first = await startExecution({
      dbPath,
      wasmPath,
      artifactJson: artifact,
      autoDeliverEvent: false,
      persist,
    });
    assert.equal(first.status, "suspended");
    return resumeExecution({
      dbPath,
      wasmPath,
      eventPayload: "ok",
      persist,
    });
  }
  const naive = await run("naive");
  const optimized = await run("optimized");
  assert.equal(naive.status, "completed");
  assert.equal(optimized.status, "completed");
  assert.deepEqual(optimized.result, naive.result);
});

const INVOKE = `
import { invoke } from "@trigora/sdk";
export default async function run() {
  const result = await invoke("child");
  return result;
}
`;

const CHILD = `
import { effect } from "@trigora/sdk";
export default async function run() {
  const result = await effect("child_work", async () => 7);
  return result;
}
`;

function childCount(store: Store): number {
  return Number((store.db.prepare("SELECT COUNT(*) AS n FROM children").get() as { n: number }).n);
}

function invokeInputs(executionIds: string[], parentJson: string, childJson: string, dbPath = "unused") {
  return executionIds.map((executionId) => ({
    dbPath,
    artifactJson: parentJson,
    executionId,
    childArtifacts: { child: childJson },
    effects: { child_work: 7 },
  }));
}

test("batch invoke completes and shares a durability batch with independent work", async () => {
  await ensureEngineLoaded(wasmPath);
  const store = new Store(dbFile(), "optimized");
  const parentJson = canonicalStringify(compile(INVOKE, { filename: "parent.ts" }));
  const childJson = canonicalStringify(compile(CHILD, { filename: "child.ts" }));
  const independent = artifactJson();
  try {
    const results = runBatchOnStore(store, [
      ...invokeInputs(["invoke-a", "invoke-b"], parentJson, childJson),
      { dbPath: "unused", artifactJson: independent, executionId: "solo", effects: { generate: 42 } },
    ]);
    assert.deepEqual(results.map((result) => result.status), ["completed", "completed", "completed"]);
    assert.deepEqual(results[0]?.result, { t: "number", v: 7 });
    assert.deepEqual(results[1]?.result, { t: "number", v: 7 });
    assert.equal(childCount(store), 2);
    assert.ok(store.metrics().batchOccupancySamples.some((count) => count >= 2));
  } finally {
    store.close();
  }
});

test("crash before invoke batch commit leaves no child", async () => {
  const dbPath = dbFile();
  const parentJson = canonicalStringify(compile(INVOKE, { filename: "parent.ts" }));
  const childJson = canonicalStringify(compile(CHILD, { filename: "child.ts" }));
  const hostUrl = new URL("./host.ts", import.meta.url).href;
  const storeUrl = new URL("./store.ts", import.meta.url).href;
  const script = `import { ensureEngineLoaded, runBatchOnStore } from ${JSON.stringify(hostUrl)};
import { Store } from ${JSON.stringify(storeUrl)};
await ensureEngineLoaded(process.env.TCC_WASM);
const store = new Store(process.env.TCC_DB, "optimized");
runBatchOnStore(store, [{
  dbPath: process.env.TCC_DB, artifactJson: process.env.TCC_PARENT, executionId: "p1",
  childArtifacts: { child: process.env.TCC_CHILD }, effects: { child_work: 7 },
}], () => { process.kill(process.pid, "SIGKILL"); });`;
  const child = spawnSync(process.execPath, ["--experimental-sqlite", "--experimental-strip-types", "--input-type=module", "-e", script], {
    env: { ...process.env, TCC_DB: dbPath, TCC_WASM: wasmPath, TCC_PARENT: parentJson, TCC_CHILD: childJson },
  });
  assert.equal(child.signal, "SIGKILL", child.stderr.toString());
  const store = new Store(dbPath, "optimized");
  try {
    assert.equal(store.getExecution("p1")?.revision, 0);
    assert.equal(store.getContinuation("p1"), undefined);
    assert.equal(childCount(store), 0);
  } finally {
    store.close();
  }
});

test("commit then crash before parent ack keeps the same child", async () => {
  const dbPath = dbFile();
  const parentJson = canonicalStringify(compile(INVOKE, { filename: "parent.ts" }));
  const childJson = canonicalStringify(compile(CHILD, { filename: "child.ts" }));
  const hostUrl = new URL("./host.ts", import.meta.url).href;
  const storeUrl = new URL("./store.ts", import.meta.url).href;
  const script = `import { ensureEngineLoaded, runBatchOnStore } from ${JSON.stringify(hostUrl)};
import { Store } from ${JSON.stringify(storeUrl)};
await ensureEngineLoaded(process.env.TCC_WASM);
const store = new Store(process.env.TCC_DB, "optimized");
runBatchOnStore(store, [{
  dbPath: process.env.TCC_DB, artifactJson: process.env.TCC_PARENT, executionId: "p1",
  childArtifacts: { child: process.env.TCC_CHILD }, effects: { child_work: 7 },
}], undefined, () => { process.kill(process.pid, "SIGKILL"); });`;
  const child = spawnSync(process.execPath, ["--experimental-sqlite", "--experimental-strip-types", "--input-type=module", "-e", script], {
    env: { ...process.env, TCC_DB: dbPath, TCC_WASM: wasmPath, TCC_PARENT: parentJson, TCC_CHILD: childJson },
  });
  assert.equal(child.signal, "SIGKILL", child.stderr.toString());
  const store = new Store(dbPath, "optimized");
  try {
    assert.equal(store.getContinuation("p1")?.revision, 1);
    assert.equal(childCount(store), 1);
    const row = store.db.prepare("SELECT * FROM children").get() as { child_execution_id: string; invoke_id: string };
    assert.ok(store.getExecution(row.child_execution_id));
  } finally {
    store.close();
  }
  const resumed = await resumeExecution({
    dbPath,
    artifactJson: parentJson,
    executionId: "p1",
    childArtifacts: { child: childJson },
    effects: { child_work: 7 },
    persist: "optimized",
  });
  assert.equal(resumed.status, "completed");
  assert.deepEqual(resumed.result, { t: "number", v: 7 });
  const after = new Store(dbPath, "optimized");
  try {
    assert.equal(childCount(after), 1);
  } finally {
    after.close();
  }
});

test("duplicate invoke retry does not create a second child", async () => {
  await ensureEngineLoaded(wasmPath);
  const dbPath = dbFile();
  const parentJson = canonicalStringify(compile(INVOKE, { filename: "parent.ts" }));
  const childJson = canonicalStringify(compile(CHILD, { filename: "child.ts" }));
  const first = await startExecution({
    dbPath,
    wasmPath,
    artifactJson: parentJson,
    executionId: "dup",
    childArtifacts: { child: childJson },
    effects: { child_work: 7 },
    persist: "optimized",
  });
  assert.equal(first.status, "completed");
  const again = await resumeExecution({
    dbPath,
    wasmPath,
    artifactJson: parentJson,
    executionId: "dup",
    childArtifacts: { child: childJson },
    effects: { child_work: 7 },
    persist: "optimized",
  });
  assert.equal(again.status, "completed");
  const check = new Store(dbPath, "optimized");
  try {
    assert.equal(childCount(check), 1);
  } finally {
    check.close();
  }
});

test("replay persist skips continuation writes and resumes by prefix replay", async () => {
  await ensureEngineLoaded(wasmPath);
  const dbPath = dbFile();
  const artifact = artifactJson();
  const first = await startExecution({
    dbPath,
    wasmPath,
    artifactJson: artifact,
    persist: "replay",
    autoDeliverEvent: false,
  });
  assert.equal(first.status, "suspended");
  const store = new Store(dbPath, "replay");
  try {
    assert.equal(store.getContinuation("first"), undefined);
    assert.equal(store.getExecution("first")?.revision, 2);
    const history = store.walRecords("first");
    assert.ok(history.every((row) => row.kind === "history"));
  } finally {
    store.close();
  }
  const resumed = await resumeExecution({
    dbPath,
    wasmPath,
    artifactJson: artifact,
    persist: "replay",
    eventPayload: "ok",
  });
  assert.equal(resumed.status, "completed");
  assert.deepEqual(resumed.result, completed);
});

test("SIGKILL during replay persist recovers without re-invoking the effect", async () => {
  const dbPath = dbFile();
  const artifact = artifactJson();
  const hostUrl = new URL("./host.ts", import.meta.url).href;
  const logPath = path.join(path.dirname(dbPath), "effects.log");
  const script = `import { ensureEngineLoaded, startExecution } from ${JSON.stringify(hostUrl)};
await ensureEngineLoaded(process.env.TCC_WASM);
await startExecution({
  dbPath: process.env.TCC_DB, artifactJson: process.env.TCC_ARTIFACT, persist: "replay",
  effectLogPath: process.env.TCC_LOG, autoDeliverEvent: false,
});`;
  const child = spawnSync(process.execPath, ["--experimental-sqlite", "--experimental-strip-types", "--input-type=module", "-e", script], {
    env: { ...process.env, TCC_DB: dbPath, TCC_WASM: wasmPath, TCC_ARTIFACT: artifact, TCC_LOG: logPath, TCC_CRASH_AT: "after_persist_checkpoint:1" },
  });
  assert.equal(child.signal, "SIGKILL", child.stderr.toString());
  const resumed = await resumeExecution({
    dbPath,
    wasmPath,
    artifactJson: artifact,
    persist: "replay",
    effectLogPath: logPath,
    eventPayload: "ok",
  });
  assert.equal(resumed.status, "completed");
  const log = readFileSync(logPath, "utf8").trim().split("\n").filter(Boolean);
  assert.deepEqual(log, ["generate"]);
});

const FORBIDDEN_EVENT_KEYS = ["json", "continuationJson", "artifactJson", "payload", "resultJson", "result_json"];

function assertHostEventsClean(events: HostEvent[]): void {
  for (const event of events) {
    for (const key of FORBIDDEN_EVENT_KEYS) {
      assert.equal((event as Record<string, unknown>)[key], undefined, `${event.type} leaked ${key}`);
    }
    assert.equal(typeof event.engineVersion, "string");
    assert.ok(event.engineVersion.length > 0);
  }
}

test("host onEvent reports persist restore journal and child without blocking", async () => {
  await ensureEngineLoaded(wasmPath);
  const events: HostEvent[] = [];
  const store = new Store(dbFile(), "optimized", (event) => {
    events.push(event);
  });
  const parent = canonicalStringify(compile(INVOKE, { filename: "invoke.ts" }));
  const child = canonicalStringify(compile(CHILD, { filename: "child.ts" }));
  try {
    const result = runOnStore(store, {
      dbPath: "unused",
      artifactJson: parent,
      executionId: "obs-parent",
      childArtifacts: { child },
      effects: { child_work: 7 },
    });
    assert.equal(result.status, "completed");
    const types = events.map((event) => event.type);
    assert.ok(types.includes("checkpoint.persisted"));
    assert.ok(types.includes("checkpoint.materialized"));
    assert.ok(types.includes("batch.committed"));
    assert.ok(types.includes("child.created"));
    assert.ok(types.includes("child.completed"));
    store.getContinuation("obs-parent", { observeRestore: true });
    assert.ok(events.some((event) => event.type === "continuation.restored"));
    assertHostEventsClean(events);
  } finally {
    store.close();
  }

  const journalEvents: HostEvent[] = [];
  const journalStore = new Store(dbFile(), "optimized", (event) => {
    journalEvents.push(event);
  });
  const first = artifactJson();
  const hash = (JSON.parse(first) as { envelope: { artifact_hash: string } }).envelope.artifact_hash;
  try {
    journalStore.putArtifact(hash, first);
    journalStore.createExecution("obs-journal", hash, "owner-1", Date.now() + 60_000);
    journalStore.completeEffect("obs-journal", "generate", "k", JSON.stringify({ t: "number", v: 42 }));
    const journaled = runOnStore(journalStore, {
      dbPath: "unused",
      artifactJson: first,
      executionId: "obs-journal",
      effects: { generate: 42 },
    });
    assert.equal(journaled.status, "completed");
    assert.ok(journalEvents.some((event) => event.type === "effect.journal_hit"));
    assertHostEventsClean(journalEvents);
  } finally {
    journalStore.close();
  }

  const throwing = new Store(dbFile(), "optimized", () => {
    throw new Error("observer boom");
  });
  try {
    const survived = runOnStore(throwing, {
      dbPath: "unused",
      artifactJson: artifactJson(),
      executionId: "obs-throw",
      effects: { generate: 42 },
    });
    assert.equal(survived.status, "completed");
  } finally {
    throwing.close();
  }

  const errors: HostEvent[] = [];
  const failStore = new Store(dbFile(), "optimized", (event) => {
    errors.push(event);
  });
  try {
    failStore.putArtifact("h", "{}");
    failStore.createExecution("obs-fail", "h", "owner-1", Date.now() + 60_000);
    assert.throws(() => {
      failStore.commitCheckpoint("obs-fail", 2, dummyContinuation("obs-fail", 2), "runnable", "owner-1", {
        kind: "snapshot",
        materialize: true,
      });
    }, /revision conflict/);
    assert.ok(errors.some((event) => event.type === "runtime.error"));
    assertHostEventsClean(errors);
  } finally {
    failStore.close();
  }
});

function compileSource(source: string, filename = "live.ts"): string {
  return canonicalStringify(compile(source, { filename }));
}

function loadHead(dbPath: string): { json: string; parsed: ContinuationJson; records: Array<{ kind: string; payload: string }> } {
  const store = new Store(dbPath, "optimized");
  try {
    const saved = store.getContinuation("first");
    assert.ok(saved);
    return {
      json: saved.json,
      parsed: JSON.parse(saved.json) as ContinuationJson,
      records: store.walRecords("first"),
    };
  } finally {
    store.close();
  }
}

async function suspendOnWait(source: string, effects: Record<string, unknown>, filename = "live.ts") {
  const dbPath = dbFile();
  const artifactJson = compileSource(source, filename);
  const result = await startExecution({
    dbPath,
    wasmPath,
    artifactJson,
    effects,
    autoDeliverEvent: false,
  });
  assert.equal(result.status, "suspended", result);
  return { dbPath, artifactJson };
}

test("reconstruct golden patches a dead slot to undefined without resurrecting it", () => {
  const fixture = JSON.parse(readFileSync(path.join(fixturesDir, "slot-undefined.json"), "utf8")) as {
    base: ContinuationJson;
    delta: Parameters<typeof applyDelta>[1];
    expected: ContinuationJson;
  };
  const applied = applyDelta(fixture.base, fixture.delta);
  applied.revision = fixture.expected.revision;
  assert.deepEqual(applied, fixture.expected);
  assert.equal(JSON.stringify(applied).includes("DEAD_SHOULD_NOT_RESURRECT"), false);
  const omitted = structuredClone(fixture.base);
  omitted.frames[0]!.pc = 8;
  assert.equal(JSON.stringify(omitted).includes("DEAD_SHOULD_NOT_RESURRECT"), true);
});

test("dead scalar is undefined after a later wait; live value remains", async () => {
  const source = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const dead = await effect("dead", async () => "dead-scalar");
  const live = await effect("live", async () => "live-scalar");
  await waitForEvent("go");
  return live;
}
`;
  const { dbPath } = await suspendOnWait(source, { dead: "dead-scalar", live: "live-scalar" });
  const head = loadHead(dbPath);
  const locals = JSON.stringify(head.parsed.frames[0]?.locals ?? []);
  assert.equal(locals.includes("dead-scalar"), false);
  assert.equal(locals.includes("live-scalar"), true);
});

test("large dead string is absent from the committed continuation", async () => {
  const marker = "DEADBLOB_MARKER";
  const blob = marker + "x".repeat(1_000_000);
  const source = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const blob = await effect("blob", async () => "unused");
  await waitForEvent("go");
  return 1;
}
`;
  const { dbPath } = await suspendOnWait(source, { blob });
  const head = loadHead(dbPath);
  assert.equal(head.json.includes(marker), false);
  assert.ok(head.json.length < 50_000, `continuation still large: ${head.json.length}`);
  const snapshot = head.records.find((row) => row.kind === "snapshot");
  assert.ok(snapshot);
  assert.equal(snapshot.payload.includes(marker), true);
  assert.ok(snapshot.payload.length > 1_000_000);
  assert.ok(head.json.length < snapshot.payload.length / 2);
});

test("value live across several waits remains", async () => {
  const source = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const live = await effect("live", async () => "stay-live");
  await waitForEvent("a");
  await waitForEvent("b");
  return live;
}
`;
  const { dbPath, artifactJson } = await suspendOnWait(source, { live: "stay-live" });
  assert.equal(loadHead(dbPath).json.includes("stay-live"), true);
  const second = await resumeExecution({
    dbPath,
    wasmPath,
    artifactJson,
    eventPayload: "ok",
    autoDeliverEvent: false,
  });
  assert.equal(second.status, "suspended");
  assert.equal(loadHead(dbPath).json.includes("stay-live"), true);
});

test("dead after taken branch is gone; live arm value stays", async () => {
  const source = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const flag = await effect("flag", async () => 1);
  if (flag) {
    const taken = await effect("taken", async () => 42);
    const dead = await effect("dead", async () => "arm-dead");
    await waitForEvent("go");
    return taken;
  } else {
    const skipped = await effect("skipped", async () => 99);
    await waitForEvent("go");
    return skipped;
  }
}
`;
  const { dbPath } = await suspendOnWait(source, { flag: 1, taken: 42, dead: "arm-dead", skipped: 99 });
  const locals = JSON.stringify(loadHead(dbPath).parsed.frames[0]?.locals ?? []);
  assert.equal(locals.includes("arm-dead"), false);
  assert.equal(locals.includes('"v":42'), true);
  assert.equal(locals.includes('"v":99'), false);
});

test("loop-carried value remains at the wait inside the loop", async () => {
  const source = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  let acc = await effect("start", async () => "loop-acc");
  let go = await effect("go", async () => 1);
  while (go) {
    const tmp = await effect("tmp", async () => "loop-tmp");
    await waitForEvent("tick");
    go = 0;
  }
  return acc;
}
`;
  const { dbPath } = await suspendOnWait(source, { start: "loop-acc", go: 1, tmp: "loop-tmp" });
  const json = loadHead(dbPath).json;
  assert.equal(json.includes("loop-acc"), true);
  assert.equal(json.includes("loop-tmp"), false);
});

test("try/catch keeps a catch-live value across persist in try; finally drops dead locals", async () => {
  const keepSource = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  let keep = "";
  try {
    keep = await effect("keep", async () => "keep-me");
    await waitForEvent("in-try");
    throw "boom";
  } catch (e) {
    return keep;
  }
}
`;
  const { dbPath: keepDb } = await suspendOnWait(keepSource, { keep: "keep-me" }, "keep.ts");
  assert.equal(loadHead(keepDb).json.includes("keep-me"), true);

  const goneSource = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  try {
    const gone = await effect("gone", async () => "drop-me");
  } catch (e) {
    return e;
  } finally {
    await waitForEvent("f");
  }
  return 1;
}
`;
  const { dbPath: goneDb } = await suspendOnWait(goneSource, { gone: "drop-me" }, "gone.ts");
  assert.equal(loadHead(goneDb).json.includes("drop-me"), false);
});

test("SIGKILL after compaction resumes the live result", async () => {
  const source = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const dead = await effect("dead", async () => "dead-scalar");
  const live = await effect("live", async () => "live-scalar");
  await waitForEvent("go");
  return live;
}
`;
  const recovered = await killAndResume({
    source,
    crashAt: "after_wait_checkpoint",
    effects: { dead: "dead-scalar", live: "live-scalar" },
    eventPayload: "ok",
  });
  assert.equal(recovered.resumed.status, "completed");
  assert.deepEqual(recovered.resumed.result, { t: "string", v: "live-scalar" });
});

