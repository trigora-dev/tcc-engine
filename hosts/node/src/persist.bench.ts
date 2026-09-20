/**
 * Internal naive vs optimized persist benches. Not product numbers.
 *
 *   pnpm --filter @tcc-engine/host-node bench:persist
 *
 * Reports host, storage, live-state size, retained WAL, healthy-path
 * throughput, and recovery vs unrelated WAL growth / live-state size.
 */
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import { startExecution } from "./host.ts";
import { Store, type PersistMode } from "./store.ts";

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

const LARGE = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const a = await effect("a", async () => 1);
  const b = await effect("b", async () => 2);
  const c = await effect("c", async () => 3);
  const d = await effect("d", async () => 4);
  const approval = await waitForEvent("approved");
  return { a, b, c, d, approval };
}
`;

function dbFile(): string {
  return path.join(mkdtempSync(path.join(tmpdir(), "tcc-bench-")), "tcc.db");
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

async function healthyPath(label: string, source: string, persist: PersistMode, iters: number) {
  const artifact = canonicalStringify(compile(source, { filename: `${label}.ts` }));
  const started = performance.now();
  for (let i = 0; i < iters; i++) {
    const result = await startExecution({
      dbPath: dbFile(),
      wasmPath,
      artifactJson: artifact,
      persist,
      executionId: `exec-${i}`,
      effects: label === "first-example" ? { generate: 42 } : { a: 1, b: 2, c: 3, d: 4 },
    });
    if (result.status !== "completed") {
      throw new Error(`${label} ${persist} status ${result.status}`);
    }
  }
  const ms = performance.now() - started;
  return {
    host: "node",
    storage: persist,
    workload: label,
    iters,
    ms: Number(ms.toFixed(2)),
    perSec: Number((iters / (ms / 1000)).toFixed(2)),
  };
}

function recoveryVsForeignWal(foreignCount: number) {
  const dbPath = dbFile();
  const store = new Store(dbPath, "optimized");
  store.putArtifact("hash", "{}");
  store.beginGroup();
  for (let i = 0; i < foreignCount; i++) {
    const id = `foreign-${i}`;
    store.createExecution(id, "hash", "owner-1", Date.now() + 60_000);
    store.commitCheckpoint(id, 1, dummy(id, 1, 4), "runnable", "owner-1", {
      kind: "snapshot",
      materialize: true,
    });
  }
  store.createExecution("target", "hash", "owner-1", Date.now() + 60_000);
  store.commitCheckpoint("target", 1, dummy("target", 1, 4), "runnable", "owner-1", {
    kind: "snapshot",
    materialize: true,
  });
  store.endGroup();
  const started = performance.now();
  const saved = store.getContinuation("target");
  const ms = performance.now() - started;
  const plan = store.recoverPlan("target");
  const retainedWal = store.retainedWalBytes();
  const ownWal = store.retainedWalBytes("target");
  store.close();
  if (!saved || !plan) {
    throw new Error("missing target continuation");
  }
  return {
    host: "node",
    storage: "optimized",
    foreignCount,
    recoverMs: Number(ms.toFixed(4)),
    suffixLength: plan.suffixLength,
    usedIndex: plan.usedIndex,
    retainedWalBytes: retainedWal,
    liveStateBytes: ownWal,
  };
}

function recoveryVsLiveState(locals: number) {
  const dbPath = dbFile();
  const store = new Store(dbPath, "optimized");
  store.putArtifact("hash", "{}");
  store.createExecution("live", "hash", "owner-1", Date.now() + 60_000);
  store.commitCheckpoint("live", 1, dummy("live", 1, locals), "runnable", "owner-1", {
    kind: "snapshot",
    materialize: true,
  });
  const started = performance.now();
  const saved = store.getContinuation("live");
  const ms = performance.now() - started;
  const liveStateBytes = saved ? saved.json.length : 0;
  const retainedWal = store.retainedWalBytes("live");
  store.close();
  return {
    host: "node",
    storage: "optimized",
    locals,
    recoverMs: Number(ms.toFixed(4)),
    liveStateBytes,
    retainedWalBytes: retainedWal,
  };
}

const rows = [];
rows.push(await healthyPath("first-example", FIRST, "naive", 20));
rows.push(await healthyPath("first-example", FIRST, "optimized", 20));
rows.push(await healthyPath("larger-locals", LARGE, "naive", 20));
rows.push(await healthyPath("larger-locals", LARGE, "optimized", 20));
rows.push(recoveryVsForeignWal(50));
rows.push(recoveryVsForeignWal(400));
rows.push(recoveryVsLiveState(4));
rows.push(recoveryVsLiveState(256));

console.log("tcc persist bench (internal; not a paper claim)");
console.log(JSON.stringify(rows, null, 2));
