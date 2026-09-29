// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

import { Store } from "../hosts/node/src/store.ts";
import { buildMeta, printReport } from "./lib/report.ts";
import { benchScale } from "./lib/scale.ts";
import { latencyStats, throughputPerSec } from "./lib/stats.ts";
import { dbFile, wasmPath } from "./lib/store_metrics.ts";
import { dummyContinuation, dummyDelta } from "./lib/workloads.ts";

/** Fixed live state; grow target revisions and measure reconstruct. */
export async function runRecoveryHistoryDepth(): Promise<unknown[]> {
  const scale = benchScale();
  const meta = buildMeta({ host: "node", wasmPath: wasmPath(), scale: scale.name });
  const rows: unknown[] = [];
  const locals = 8;

  for (const revisions of scale.historyRevisions) {
    const store = new Store(dbFile(), "optimized");
    try {
      store.putArtifact("bench-hash", "{}");
      store.createExecution("target", "bench-hash", "owner-1", Date.now() + 60_000);
      store.beginGroup();
      for (let revision = 1; revision <= revisions; revision++) {
        const isSnapshot = revision === 1 || (revision - 1) % 32 === 0;
        store.commitCheckpoint(
          "target",
          revision,
          dummyContinuation("target", revision, locals),
          "runnable",
          "owner-1",
          {
            kind: isSnapshot ? "snapshot" : "delta",
            materialize: isSnapshot,
            deltaJson: isSnapshot ? null : dummyDelta(revision),
          },
        );
      }
      store.endGroup();
      const plan = store.recoverPlan("target");
      if (!plan) {
        throw new Error("missing recover plan");
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
        scenario: "recovery_history_depth",
        revisions,
        locals,
        suffixLength: plan.suffixLength,
        snapshotRevision: plan.snapshotRevision,
        usedIndex: plan.usedIndex,
        ...latency,
        recoversPerSec: throughputPerSec(scale.recoverSamples, samplesMs.reduce((a, b) => a + b, 0)),
        liveStateBytes: store.getContinuation("target")?.json.length ?? 0,
        retainedWalBytes: store.retainedWalBytes("target"),
      });
    } finally {
      store.close();
    }
  }

  printReport(
    "recovery_history_depth",
    meta,
    rows,
  );
  return rows;
}
