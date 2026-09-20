import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import { resumeExecution, startExecution } from "./host.ts";
import {
  applyDelta,
  MATERIALIZE_EVERY,
  reconstructContinuation,
  Store,
  type ContinuationJson,
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

test("reconstruct goldens match applyDelta", () => {
  const files = readdirSync(fixturesDir).filter((name) => name.endsWith(".json"));
  assert.ok(files.length >= 3);
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
