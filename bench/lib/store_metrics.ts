import path from "node:path";
import { fileURLToPath } from "node:url";
import { existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";

import type { StoreMetrics } from "../../hosts/node/src/store.ts";
import { summarizeOccupancy } from "./stats.ts";

export function wasmPath(): string {
  return path.resolve(
    fileURLToPath(new URL("../../target/wasm32-unknown-unknown/release/tcc_wasm.wasm", import.meta.url)),
  );
}

/** Fail fast if the release WASM artifact is missing. Benches do not rebuild it. */
export function requireWasm(): string {
  const artifact = wasmPath();
  if (!existsSync(artifact)) {
    console.error(`missing release WASM at ${artifact}`);
    console.error("build it with: pnpm build:wasm");
    process.exit(1);
  }
  return artifact;
}

export function dbFile(prefix = "tcc-bench-"): string {
  return path.join(mkdtempSync(path.join(tmpdir(), prefix)), "tcc.db");
}

export function persistAccounting(metrics: StoreMetrics) {
  const occupancy = summarizeOccupancy(metrics.batchOccupancySamples);
  const avgDeltaSize =
    metrics.deltaCount === 0
      ? 0
      : Number((metrics.deltaBytesWritten / metrics.deltaCount).toFixed(2));
  const deltaPayloadShare =
    metrics.bytesWritten === 0
      ? 0
      : Number((metrics.deltaBytesWritten / metrics.bytesWritten).toFixed(4));
  return {
    bytesWritten: metrics.bytesWritten,
    snapshotCount: metrics.snapshotCount,
    deltaCount: metrics.deltaCount,
    averageDeltaSize: avgDeltaSize,
    commitCount: metrics.commitCount,
    batchOccupancyAvg: occupancy.avg,
    batchOccupancyMax: occupancy.max,
    batchFlushCount: occupancy.count,
    deltaPayloadShare,
  };
}
