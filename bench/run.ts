#!/usr/bin/env node
/**
 * Public persistence benchmark suite (Node/WASM reference host).
 *
 *   pnpm bench
 *   pnpm bench:persistence
 *   pnpm bench:recovery
 *   TCC_BENCH_SCALE=quick pnpm bench
 *
 * Optional targeted aliases:
 *   pnpm bench:persistence:healthy
 *   pnpm bench:persistence:group
 *   pnpm bench:persistence:replay
 *   pnpm bench:recovery:history
 *   pnpm bench:recovery:live-state
 *   pnpm bench:recovery:wal
 */
import { runConcurrencyGroupCommit } from "./concurrency_group_commit.ts";
import { runHealthyPathPersistence } from "./healthy_path_persistence.ts";
import { runHealthyPathReplayBaseline } from "./healthy_path_replay_baseline.ts";
import { runLiveStateScaling } from "./live_state_scaling.ts";
import { DISCLAIMER } from "./lib/scale.ts";
import { requireWasm } from "./lib/store_metrics.ts";
import { runRecoveryHistoryDepth } from "./recovery_history_depth.ts";
import { runWalIsolation } from "./wal_isolation.ts";

const ALIASES: Record<string, string> = {
  concurrency: "persistence:group",
  replay: "persistence:replay",
  isolation: "recovery:wal",
};

const USAGE =
  "usage: node bench/run.ts [all|persistence|recovery|persistence:healthy|persistence:group|persistence:replay|recovery:history|recovery:live-state|recovery:wal]";

const known = new Set([
  "all",
  "persistence",
  "recovery",
  "persistence:healthy",
  "persistence:group",
  "persistence:replay",
  "recovery:history",
  "recovery:live-state",
  "recovery:wal",
]);

const requested = process.argv[2] ?? "all";
const mode = ALIASES[requested] ?? requested;

if (!known.has(mode)) {
  console.error(`unknown mode: ${requested}`);
  console.error(USAGE);
  process.exit(1);
}

requireWasm();

console.log(DISCLAIMER);

const all: unknown[] = [];

if (mode === "all" || mode === "persistence" || mode === "persistence:healthy") {
  all.push(...(await runHealthyPathPersistence()));
}
if (mode === "all" || mode === "persistence" || mode === "persistence:group") {
  all.push(...(await runConcurrencyGroupCommit()));
}
if (mode === "all" || mode === "persistence" || mode === "persistence:replay") {
  all.push(...(await runHealthyPathReplayBaseline()));
}

if (mode === "all" || mode === "recovery" || mode === "recovery:history") {
  all.push(...(await runRecoveryHistoryDepth()));
}
if (mode === "all" || mode === "recovery" || mode === "recovery:live-state") {
  all.push(...(await runLiveStateScaling()));
}
if (mode === "all" || mode === "recovery" || mode === "recovery:wal") {
  all.push(...(await runWalIsolation()));
}

console.log(`\n=== suite complete (${mode}) rows=${all.length} ===`);
