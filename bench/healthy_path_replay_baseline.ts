// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

import { ensureEngineLoaded, runBatchOnStore, runOnStore } from "../hosts/node/src/host.ts";
import { Store, type PersistMode } from "../hosts/node/src/store.ts";
import { buildMeta, printReport } from "./lib/report.ts";
import { benchScale } from "./lib/scale.ts";
import { latencyStats, relativeOverhead, throughputPerSec, timedLoop } from "./lib/stats.ts";
import { dbFile, persistAccounting, wasmPath } from "./lib/store_metrics.ts";
import { loadWorkloads } from "./lib/workloads.ts";

type ReplayStorage = Extract<PersistMode, "replay" | "optimized">;

type ReplayRow = {
  scenario: "healthy_path_replay_baseline";
  workload: string;
  storage: ReplayStorage;
  concurrency: number;
  coordination: "single" | "wave";
  executionsPerSec: number;
  overheadVsReplay?: number | null;
  [key: string]: unknown;
};

function rowKey(row: Pick<ReplayRow, "workload" | "concurrency" | "coordination">): string {
  return `${row.workload}:${row.concurrency}:${row.coordination}`;
}

function attachOverhead(rows: ReplayRow[]): void {
  const replay = new Map<string, number>();
  for (const row of rows) {
    if (row.storage === "replay") {
      replay.set(rowKey(row), Number(row.medianMs));
    }
  }
  for (const row of rows) {
    if (row.storage !== "optimized") {
      continue;
    }
    row.overheadVsReplay = relativeOverhead(Number(row.medianMs), replay.get(rowKey(row)) ?? 0);
  }
}

export async function runHealthyPathReplayBaseline(): Promise<unknown[]> {
  const scale = benchScale();
  const wasm = wasmPath();
  const meta = buildMeta({ host: "node", wasmPath: wasm, scale: scale.name });
  await ensureEngineLoaded(wasm);
  const workloads = loadWorkloads();
  const workloadA = workloads[0]!;
  const rows: ReplayRow[] = [];
  const models: ReplayStorage[] = ["replay", "optimized"];

  for (const workload of workloads) {
    for (const persist of models) {
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
          scenario: "healthy_path_replay_baseline",
          workload: workload.id,
          storage: persist,
          concurrency: 1,
          coordination: "single",
          ...latency,
          executionsPerSec: throughputPerSec(scale.measured, totalMeasuredMs),
          ...persistAccounting(store.metrics()),
        });
      } finally {
        store.close();
      }
    }
  }

  const waves = Math.max(1, Math.floor(scale.measured / Math.max(...scale.concurrencyLevels)));
  for (const persist of models) {
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
            artifactJson: workloadA.artifactJson,
            executionId: `r-${concurrency}-w${w}-n${n}`,
            effects: workloadA.effects,
            persist,
            isolatedEngine: true,
          }));
          const results = runBatchOnStore(store, inputs);
          for (const result of results) {
            if (result.status !== "completed") {
              throw new Error(`replay baseline concurrency ${concurrency} status ${result.status}`);
            }
            completed += 1;
          }
          samplesMs.push(performance.now() - t0);
        }
        const latency = latencyStats(samplesMs);
        const totalMs = samplesMs.reduce((a, b) => a + b, 0);
        rows.push({
          scenario: "healthy_path_replay_baseline",
          workload: workloadA.id,
          storage: persist,
          concurrency,
          coordination: "wave",
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

  attachOverhead(rows);
  printReport("healthy_path_replay_baseline", meta, rows);
  return rows;
}
