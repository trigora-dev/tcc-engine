// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

import { Store } from "../hosts/node/src/store.ts";
import { buildMeta, printReport } from "./lib/report.ts";
import { benchScale } from "./lib/scale.ts";
import { latencyStats, throughputPerSec } from "./lib/stats.ts";
import { dbFile, wasmPath } from "./lib/store_metrics.ts";
import { dummyContinuation, dummyDelta } from "./lib/workloads.ts";

/**
 * Unrelated WAL growth with a fixed non-zero target suffix (snapshot + N deltas).
 * Sub-ms recover times may move with SQLite cache noise; usedIndex + suffixLength are the hard checks.
 */
export async function runWalIsolation(): Promise<unknown[]> {
  const scale = benchScale();
  const meta = buildMeta({ host: "node", wasmPath: wasmPath(), scale: scale.name });
  const rows: unknown[] = [];
  const suffix = scale.targetSuffix;

  for (const foreignCount of scale.foreignWalSizes) {
    const store = new Store(dbFile(), "optimized");
    try {
      store.putArtifact("bench-hash", "{}");
      store.beginGroup();
      for (let i = 0; i < foreignCount; i++) {
        const id = `foreign-${i}`;
        store.createExecution(id, "bench-hash", "owner-1", Date.now() + 60_000);
        store.commitCheckpoint(id, 1, dummyContinuation(id, 1, 4), "runnable", "owner-1", {
          kind: "snapshot",
          materialize: true,
        });
      }
      store.createExecution("target", "bench-hash", "owner-1", Date.now() + 60_000);
      store.commitCheckpoint("target", 1, dummyContinuation("target", 1, 4), "runnable", "owner-1", {
        kind: "snapshot",
        materialize: true,
      });
      for (let revision = 2; revision <= suffix + 1; revision++) {
        store.commitCheckpoint(
          "target",
          revision,
          dummyContinuation("target", revision, 4),
          "runnable",
          "owner-1",
          {
            kind: "delta",
            materialize: false,
            deltaJson: dummyDelta(revision),
          },
        );
      }
      store.endGroup();

      const plan = store.recoverPlan("target");
      if (!plan) {
        throw new Error("missing recover plan");
      }
      if (plan.suffixLength !== suffix) {
        throw new Error(`expected suffix ${suffix}, got ${plan.suffixLength}`);
      }
      if (!plan.usedIndex) {
        throw new Error(`expected indexed recovery plan, got: ${plan.detail}`);
      }

      const samplesMs: number[] = [];
      for (let i = 0; i < scale.recoverSamples; i++) {
        const t0 = performance.now();
        const saved = store.getContinuation("target");
        samplesMs.push(performance.now() - t0);
        if (!saved) {
          throw new Error("missing continuation");
        }
      }
      const latency = latencyStats(samplesMs);
      rows.push({
        scenario: "wal_isolation",
        foreignCount,
        targetSuffix: suffix,
        suffixLength: plan.suffixLength,
        usedIndex: plan.usedIndex,
        ...latency,
        recoversPerSec: throughputPerSec(scale.recoverSamples, samplesMs.reduce((a, b) => a + b, 0)),
        retainedWalBytes: store.retainedWalBytes(),
        liveStateBytes: store.retainedWalBytes("target"),
        ownWalRows: store.walRecords("target").length,
      });
    } finally {
      store.close();
    }
  }

  printReport("wal_isolation", meta, rows);
  return rows;
}
