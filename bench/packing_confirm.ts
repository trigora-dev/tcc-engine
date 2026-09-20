#!/usr/bin/env node
/**
 * Confirm a shortlist of packing policies at full bench scale.
 *
 *   node --experimental-sqlite --experimental-strip-types bench/packing_confirm.ts
 */
import { runConcurrencyGroupCommit } from "./concurrency_group_commit.ts";
import { runHealthyPathPersistence } from "./healthy_path_persistence.ts";
import { requireWasm } from "./lib/store_metrics.ts";

type Row = {
  workload?: string;
  storage?: string;
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

const CANDIDATES: Array<{ label: string; packing: string; min?: number; ratio?: number; storage: string }> = [
  { label: "naive", packing: "follow", storage: "naive" },
  { label: "follow", packing: "follow", storage: "optimized" },
  { label: "adaptive min=0 ratio=0.35", packing: "adaptive", min: 0, ratio: 0.35, storage: "optimized" },
  { label: "adaptive min=512 ratio=1", packing: "adaptive", min: 512, ratio: 1, storage: "optimized" },
];

function apply(candidate: (typeof CANDIDATES)[number]): void {
  process.env.TCC_BENCH_QUIET = "1";
  process.env.TCC_PERSIST_PACKING = candidate.packing;
  process.env.TCC_BENCH_STORAGE = candidate.storage;
  if (candidate.min === undefined) delete process.env.TCC_PERSIST_MIN_FULL_BYTES;
  else process.env.TCC_PERSIST_MIN_FULL_BYTES = String(candidate.min);
  if (candidate.ratio === undefined) delete process.env.TCC_PERSIST_MAX_DELTA_RATIO;
  else process.env.TCC_PERSIST_MAX_DELTA_RATIO = String(candidate.ratio);
}

async function main(): Promise<void> {
  requireWasm();
  for (const candidate of CANDIDATES) {
    apply(candidate);
    const healthy = (await runHealthyPathPersistence()) as Row[];
    const group = (await runConcurrencyGroupCommit()) as Row[];
    console.log(`\n# ${candidate.label}`);
    console.log(
      "healthy:",
      healthy
        .map(
          (row) =>
            `${row.workload} med=${row.medianMs} p95=${row.p95Ms} p99=${row.p99Ms} B=${row.bytesWritten} snap=${row.snapshotCount} d=${row.deltaCount} eps=${row.executionsPerSec}`,
        )
        .join(" | "),
    );
    console.log(
      "group:",
      group
        .map((row) => `c${row.concurrency} med=${row.medianMs} p95=${row.p95Ms} B=${row.bytesWritten} eps=${row.executionsPerSec}`)
        .join(" | "),
    );
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
