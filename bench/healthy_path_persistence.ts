import { ensureEngineLoaded, runOnStore } from "../hosts/node/src/host.ts";
import { Store, type PersistMode } from "../hosts/node/src/store.ts";
import { buildMeta, printReport } from "./lib/report.ts";
import { benchScale, persistModes } from "./lib/scale.ts";
import { latencyStats, throughputPerSec, timedLoop } from "./lib/stats.ts";
import { dbFile, persistAccounting, wasmPath } from "./lib/store_metrics.ts";
import { loadWorkloads } from "./lib/workloads.ts";

export async function runHealthyPathPersistence(): Promise<unknown[]> {
  const scale = benchScale();
  const wasm = wasmPath();
  const meta = buildMeta({ host: "node", wasmPath: wasm, scale: scale.name });
  await ensureEngineLoaded(wasm);
  const workloads = loadWorkloads();
  const rows: unknown[] = [];

  for (const workload of workloads) {
    for (const persist of persistModes()) {
      const store = new Store(dbFile(), persist);
      store.resetMetrics();
      try {
        const { samplesMs, totalMeasuredMs } = await timedLoop(scale.warmup, scale.measured, (i) => {
          const result = runOnStore(store, {
            dbPath: "unused",
            artifactJson: workload.artifactJson,
            executionId: `exec-${i}`,
            effects: workload.effects,
            persist,
          });
          if (result.status !== "completed") {
            throw new Error(`${workload.id} ${persist} status ${result.status}`);
          }
        }, () => store.resetMetrics());
        const latency = latencyStats(samplesMs);
        rows.push({
          scenario: "healthy_path_persistence",
          workload: workload.id,
          label: workload.label,
          shape: workload.shape,
          storage: persist,
          packing: persist === "optimized" ? store.packing : "n/a",
          packingMinFullBytes: persist === "optimized" ? store.packingThresholds.minFullBytes : null,
          packingMaxDeltaRatio: persist === "optimized" ? store.packingThresholds.maxDeltaRatio : null,
          ...latency,
          executionsPerSec: throughputPerSec(scale.measured, totalMeasuredMs),
          ...persistAccounting(store.metrics()),
        });
      } finally {
        store.close();
      }
    }
  }

  printReport("healthy_path_persistence", meta, rows);
  return rows;
}
