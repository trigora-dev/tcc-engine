// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

import { Store } from "../hosts/node/src/store.ts";
import { buildMeta, printReport } from "./lib/report.ts";
import { benchScale } from "./lib/scale.ts";
import { latencyStats, throughputPerSec } from "./lib/stats.ts";
import { dbFile, wasmPath } from "./lib/store_metrics.ts";
import { dummyContinuation } from "./lib/workloads.ts";

export async function runLiveStateScaling(): Promise<unknown[]> {
  const scale = benchScale();
  const meta = buildMeta({ host: "node", wasmPath: wasmPath(), scale: scale.name });
  const rows: unknown[] = [];

  for (const locals of scale.liveStateLocals) {
    const store = new Store(dbFile(), "optimized");
    try {
      store.putArtifact("bench-hash", "{}");
      store.createExecution("live", "bench-hash", "owner-1", Date.now() + 60_000);
      store.commitCheckpoint(
        "live",
        1,
        dummyContinuation("live", 1, locals),
        "runnable",
        "owner-1",
        { kind: "snapshot", materialize: true },
      );
      const samplesMs: number[] = [];
      let liveStateBytes = 0;
      for (let i = 0; i < scale.recoverSamples; i++) {
        const t0 = performance.now();
        const saved = store.getContinuation("live");
        samplesMs.push(performance.now() - t0);
        if (!saved) {
          throw new Error("missing continuation");
        }
        liveStateBytes = saved.json.length;
      }
      const latency = latencyStats(samplesMs);
      rows.push({
        scenario: "live_state_scaling",
        locals,
        liveStateBytes,
        retainedWalBytes: store.retainedWalBytes("live"),
        ...latency,
        recoversPerSec: throughputPerSec(scale.recoverSamples, samplesMs.reduce((a, b) => a + b, 0)),
      });
    } finally {
      store.close();
    }
  }

  printReport("live_state_scaling", meta, rows);
  return rows;
}
