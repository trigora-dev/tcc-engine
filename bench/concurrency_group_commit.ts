import { ensureEngineLoaded, runBatchOnStore, runOnStore } from "../hosts/node/src/host.ts";
import { Store, type PersistMode } from "../hosts/node/src/store.ts";
import { buildMeta, printReport } from "./lib/report.ts";
import { benchScale, persistModes } from "./lib/scale.ts";
import { latencyStats, throughputPerSec } from "./lib/stats.ts";
import { dbFile, persistAccounting, wasmPath } from "./lib/store_metrics.ts";
import { loadWorkloads } from "./lib/workloads.ts";

/**
 * `concurrency` is the number of in-flight executions in a coordinated wave,
 * not the count of parallel workers.
 */
export async function runConcurrencyGroupCommit(): Promise<unknown[]> {
  const scale = benchScale();
  const wasm = wasmPath();
  const meta = buildMeta({ host: "node", wasmPath: wasm, scale: scale.name });
  await ensureEngineLoaded(wasm);
  const workload = loadWorkloads(["A"])[0]!;
  const rows: unknown[] = [];
  const waves = Math.max(1, Math.floor(scale.measured / Math.max(...scale.concurrencyLevels)));

  for (const persist of persistModes()) {
    for (const concurrency of scale.concurrencyLevels) {
      const store = new Store(dbFile(), persist);
      store.resetMetrics();
      const samplesMs: number[] = [];
      let completed = 0;
      try {
        for (let w = 0; w < waves; w++) {
          const t0 = performance.now();
          const inputs = Array.from({ length: concurrency }, (_, n) => ({
              dbPath: "unused",
              artifactJson: workload.artifactJson,
              executionId: `c-${concurrency}-w${w}-n${n}`,
              effects: workload.effects,
              persist,
              isolatedEngine: true,
            }));
          const results = persist === "optimized"
            ? runBatchOnStore(store, inputs)
            : inputs.map((input) => {
                const result = runOnStore(store, input);
                return result;
              });
          for (const result of results) {
            if (result.status !== "completed") {
              throw new Error(`concurrency ${concurrency} status ${result.status}`);
            }
            completed += 1;
          }
          samplesMs.push(performance.now() - t0);
        }
        const latency = latencyStats(samplesMs);
        const totalMs = samplesMs.reduce((a, b) => a + b, 0);
        rows.push({
          scenario: "concurrency_group_commit",
          workload: workload.id,
          storage: persist,
          packing: persist === "optimized" ? store.packing : "n/a",
          packingMinFullBytes: persist === "optimized" ? store.packingThresholds.minFullBytes : null,
          packingMaxDeltaRatio: persist === "optimized" ? store.packingThresholds.maxDeltaRatio : null,
          concurrency,
          waves,
          completed,
          ...latency,
          executionsPerSec: throughputPerSec(completed, totalMs),
          ...persistAccounting(store.metrics()),
        });
      } finally {
        store.close();
      }
    }
  }

  printReport("concurrency_group_commit", meta, rows);
  return rows;
}
