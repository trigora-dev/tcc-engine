#!/usr/bin/env node
// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

/**
 * Internal packing-policy sweep. Not a public claim surface.
 *
 *   TCC_BENCH_SCALE=quick node --experimental-sqlite --experimental-strip-types bench/packing_sweep.ts
 */
import { runConcurrencyGroupCommit } from "./concurrency_group_commit.ts";
import { runHealthyPathPersistence } from "./healthy_path_persistence.ts";
import { requireWasm } from "./lib/store_metrics.ts";

type Row = {
  scenario?: string;
  workload?: string;
  storage?: string;
  packing?: string;
  packingMinFullBytes?: number | null;
  packingMaxDeltaRatio?: number | null;
  concurrency?: number;
  medianMs?: number;
  p95Ms?: number;
  p99Ms?: number;
  bytesWritten?: number;
  snapshotCount?: number;
  deltaCount?: number;
  deltaPayloadShare?: number;
  executionsPerSec?: number;
};

const MIN_BYTES = [0, 512, 1024, 2048, 4096];
const MAX_RATIOS = [0.35, 0.5, 0.8, 1.0];

function setPacking(packing: string, minFullBytes?: number, maxDeltaRatio?: number): void {
  process.env.TCC_PERSIST_PACKING = packing;
  process.env.TCC_BENCH_STORAGE = packing === "naive" ? "naive" : "optimized";
  process.env.TCC_BENCH_QUIET = "1";
  if (minFullBytes === undefined) {
    delete process.env.TCC_PERSIST_MIN_FULL_BYTES;
  } else {
    process.env.TCC_PERSIST_MIN_FULL_BYTES = String(minFullBytes);
  }
  if (maxDeltaRatio === undefined) {
    delete process.env.TCC_PERSIST_MAX_DELTA_RATIO;
  } else {
    process.env.TCC_PERSIST_MAX_DELTA_RATIO = String(maxDeltaRatio);
  }
}

async function collect(label: string): Promise<{ healthy: Row[]; group: Row[] }> {
  const healthy = (await runHealthyPathPersistence()) as Row[];
  const group = (await runConcurrencyGroupCommit()) as Row[];
  console.error(`collected ${label}: healthy=${healthy.length} group=${group.length}`);
  return { healthy, group };
}

function summarizeHealthy(rows: Row[]): string {
  return rows
    .map((row) => {
      return `${row.workload} med=${row.medianMs} p95=${row.p95Ms} p99=${row.p99Ms} B=${row.bytesWritten} snap=${row.snapshotCount} d=${row.deltaCount} share=${row.deltaPayloadShare} eps=${row.executionsPerSec}`;
    })
    .join(" | ");
}

function summarizeGroup(rows: Row[]): string {
  return rows
    .map((row) => `c${row.concurrency} med=${row.medianMs} p95=${row.p95Ms} B=${row.bytesWritten} eps=${row.executionsPerSec}`)
    .join(" | ");
}

async function main(): Promise<void> {
  requireWasm();
  const results: Array<{ label: string; healthy: Row[]; group: Row[] }> = [];

  setPacking("follow");
  process.env.TCC_BENCH_STORAGE = "naive";
  results.push({ label: "naive", ...(await collect("naive")) });

  setPacking("follow");
  results.push({ label: "follow", ...(await collect("follow")) });

  for (const minFullBytes of MIN_BYTES) {
    for (const maxDeltaRatio of MAX_RATIOS) {
      setPacking("adaptive", minFullBytes, maxDeltaRatio);
      const label = `adaptive min=${minFullBytes} ratio=${maxDeltaRatio}`;
      results.push({ label, ...(await collect(label)) });
    }
  }

  console.log("\n=== packing sweep ===");
  for (const result of results) {
    console.log(`\n# ${result.label}`);
    console.log(`healthy: ${summarizeHealthy(result.healthy)}`);
    console.log(`group:   ${summarizeGroup(result.group)}`);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
