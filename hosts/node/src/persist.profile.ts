// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

/** Internal sequential persistence profile; run with Node's SQLite and strip-types flags. */
import { ensureEngineLoaded, runOnStore, type PersistProfile } from "./host.ts";
import { Store, type PersistMode } from "./store.ts";
import { dbFile, wasmPath } from "../../../bench/lib/store_metrics.ts";
import { loadWorkloads } from "../../../bench/lib/workloads.ts";

const warmup = Number(process.env.TCC_PROFILE_WARMUP ?? 20);
const measured = Number(process.env.TCC_PROFILE_MEASURED ?? 100);
await ensureEngineLoaded(wasmPath());

for (const workload of loadWorkloads()) {
  for (const persist of ["naive", "optimized"] as PersistMode[]) {
    const store = new Store(dbFile("tcc-profile-"), persist);
    const profile: PersistProfile = {
      engineRequestMs: 0,
      continuationEncodeMs: 0,
      hostEncodeMs: 0,
      storeCommitMs: 0,
      persistConfirmedMs: 0,
      checkpoints: 0,
    };
    try {
      for (let i = 0; i < warmup; i++) {
        runOnStore(store, {
          dbPath: "unused", artifactJson: workload.artifactJson, executionId: `warm-${i}`,
          effects: workload.effects, persist,
        });
      }
      store.enableProfile();
      const started = performance.now();
      for (let i = 0; i < measured; i++) {
        const result = runOnStore(store, {
          dbPath: "unused", artifactJson: workload.artifactJson, executionId: `measured-${i}`,
          effects: workload.effects, persist, profile,
        });
        if (result.status !== "completed") throw new Error(`${workload.id}: ${result.status}`);
      }
      const totalMs = performance.now() - started;
      const storage = store.profileMetrics()!;
      console.log(JSON.stringify({
        workload: workload.id, persist, measured, totalMs, checkpoints: profile.checkpoints,
        perExecutionMs: Object.fromEntries(Object.entries({ ...profile, ...storage })
          .filter(([key]) => key !== "checkpoints")
          .map(([key, value]) => [key, Number((value / measured).toFixed(4))])),
      }));
    } finally {
      store.close();
    }
  }
}
