/**
 * Internal engineering persist benches (tuning / micro / adversarial).
 * Not the public claim surface — use `pnpm bench` instead.
 *
 *   pnpm --filter @tcc-engine/host-node bench:persist:internal
 *   TCC_BENCH_SCALE=quick pnpm --filter @tcc-engine/host-node bench:persist:internal
 */
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import { startExecution } from "./host.ts";
import { applyDelta, MATERIALIZE_EVERY, Store, type PersistMode } from "./store.ts";

const wasmPath = path.resolve(
  fileURLToPath(
    new URL("../../../target/wasm32-unknown-unknown/release/tcc_wasm.wasm", import.meta.url),
  ),
);

const FIRST = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const result = await effect("generate", async () => generateSomething());
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`;

function dbFile(): string {
  return path.join(mkdtempSync(path.join(tmpdir(), "tcc-internal-bench-")), "tcc.db");
}

function dummy(executionId: string, revision: number, locals: number): string {
  return JSON.stringify({
    artifact_hash: "hash",
    engine_format_version: 1,
    execution_id: executionId,
    frames: [
      {
        func_id: 0,
        locals: Array.from({ length: locals }, () => ({ t: "number", v: 1 })),
        pc: revision,
      },
    ],
    language_semantics_version: "ts.subset.v1",
    pending: null,
    result: null,
    revision,
    stack: [],
    status: "runnable",
    try_stack: [],
  });
}

/** Micro: applyDelta cost without SQLite. */
function microApplyDelta(iters: number) {
  const base = JSON.parse(dummy("micro", 0, 64));
  const delta = {
    frames: [{ index: 0, pc: 7, locals: [{ slot: 3, value: { t: "number", v: 99 } }] }],
    stack: [{ t: "string", v: "x" }],
  };
  const started = performance.now();
  for (let i = 0; i < iters; i++) {
    applyDelta(base, delta);
  }
  return {
    kind: "micro_apply_delta",
    iters,
    ms: Number((performance.now() - started).toFixed(2)),
  };
}

/** Tuning: measure delta vs snapshot payload sizes across revisions (no auto-switch). */
function tuningDeltaRatio() {
  const store = new Store(dbFile(), "optimized");
  store.putArtifact("hash", "{}");
  store.createExecution("tune", "hash", "owner-1", Date.now() + 60_000);
  const ratios: Array<{
    revision: number;
    kind: string;
    payloadBytes: number;
    snapshotBytes: number;
    deltaBytes: number;
    deltaOverSnapshot: number;
  }> = [];
  for (let revision = 1; revision <= MATERIALIZE_EVERY + 2; revision++) {
    const snapshot = dummy("tune", revision, 32);
    const deltaJson = JSON.stringify({
      frames: Array.from({ length: 16 }, (_, slot) => ({
        index: 0,
        locals: [{ slot, value: { t: "number", v: revision } }],
      })),
    });
    store.commitCheckpoint("tune", revision, snapshot, "runnable", "owner-1", {
      kind: revision === 1 ? "snapshot" : "delta",
      materialize: revision === 1,
      deltaJson,
    });
    const last = store.walRecords("tune").at(-1)!;
    ratios.push({
      revision,
      kind: last.kind,
      payloadBytes: last.payload.length,
      snapshotBytes: snapshot.length,
      deltaBytes: deltaJson.length,
      deltaOverSnapshot: Number((deltaJson.length / snapshot.length).toFixed(4)),
    });
  }
  const metrics = store.metrics();
  store.close();
  return { kind: "tuning_delta_ratio", ratios, metrics };
}

/** Adversarial: all-locals-dirty every step. */
function adversarialAllDirty() {
  const store = new Store(dbFile(), "optimized");
  store.putArtifact("hash", "{}");
  store.createExecution("adv", "hash", "owner-1", Date.now() + 60_000);
  const locals = 64;
  for (let revision = 1; revision <= 40; revision++) {
    const snapshot = dummy("adv", revision, locals);
    const deltaJson = JSON.stringify({
      frames: [
        {
          index: 0,
          pc: revision,
          locals: Array.from({ length: locals }, (_, slot) => ({
            slot,
            value: { t: "number", v: revision * 100 + slot },
          })),
        },
      ],
    });
    store.commitCheckpoint("adv", revision, snapshot, "runnable", "owner-1", {
      kind: revision === 1 ? "snapshot" : "delta",
      materialize: revision === 1,
      deltaJson,
    });
  }
  const plan = store.recoverPlan("adv");
  const metrics = store.metrics();
  store.close();
  return { kind: "adversarial_all_dirty", plan, metrics };
}

async function smokeHealthy(persist: PersistMode, iters: number) {
  const artifact = canonicalStringify(compile(FIRST, { filename: "first.ts" }));
  const started = performance.now();
  for (let i = 0; i < iters; i++) {
    const result = await startExecution({
      dbPath: dbFile(),
      wasmPath,
      artifactJson: artifact,
      persist,
      executionId: `exec-${i}`,
      effects: { generate: 42 },
    });
    if (result.status !== "completed") {
      throw new Error(result.status);
    }
  }
  return {
    kind: "internal_healthy_smoke",
    storage: persist,
    iters,
    ms: Number((performance.now() - started).toFixed(2)),
  };
}

console.log("internal persist bench — not for publication; see bench/README.md");
const rows = [
  microApplyDelta(5_000),
  tuningDeltaRatio(),
  adversarialAllDirty(),
  await smokeHealthy("naive", 10),
  await smokeHealthy("optimized", 10),
];
console.log(JSON.stringify(rows, null, 2));
