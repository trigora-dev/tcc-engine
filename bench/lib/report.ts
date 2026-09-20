import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { arch, cpus, platform, release, totalmem } from "node:os";
import { DatabaseSync } from "node:sqlite";
import { DISCLAIMER } from "./scale.ts";

export type BenchMeta = {
  disclaimer: string;
  host: string;
  runtime: string;
  wasmPath: string;
  scale: string;
  gitSha: string | null;
  engineVersion: string;
  sqliteVersion: string;
  cpuModel: string;
  memoryBytes: number;
  os: string;
  arch: string;
  startedAt: string;
  finishedAt?: string;
  durationMs?: number;
};

export function buildMeta(opts: {
  host: string;
  wasmPath: string;
  scale: string;
}): BenchMeta {
  let gitSha: string | null = null;
  try {
    gitSha = execSync("git rev-parse --short HEAD", { encoding: "utf8" }).trim();
  } catch {
    gitSha = null;
  }
  const db = new DatabaseSync(":memory:");
  const sqliteVersion = (db.prepare("SELECT sqlite_version() AS version").get() as { version: string }).version;
  db.close();
  const pkg = JSON.parse(readFileSync(new URL("../../package.json", import.meta.url), "utf8")) as { version: string };
  return {
    disclaimer: DISCLAIMER,
    host: opts.host,
    runtime: `node ${process.version}`,
    wasmPath: opts.wasmPath,
    scale: opts.scale,
    gitSha,
    engineVersion: pkg.version,
    sqliteVersion,
    cpuModel: cpus()[0]?.model ?? "unknown",
    memoryBytes: totalmem(),
    os: `${platform()} ${release()}`,
    arch: arch(),
    startedAt: new Date().toISOString(),
  };
}

export function printReport(suite: string, meta: BenchMeta, rows: unknown[]): void {
  const finishedAt = new Date().toISOString();
  const finished = { ...meta, finishedAt, durationMs: Date.parse(finishedAt) - Date.parse(meta.startedAt) };
  console.log(`\n=== ${suite} ===`);
  console.log(finished.disclaimer);
  console.log(
    `host=${finished.host} scale=${finished.scale} runtime=${finished.runtime} git=${finished.gitSha ?? "n/a"}`,
  );
  console.log("\n-- summary --");
  for (const row of rows) {
    console.log(JSON.stringify(row));
  }
  console.log("\n-- json --");
  console.log(JSON.stringify({ meta: finished, results: rows }, null, 2));
}
