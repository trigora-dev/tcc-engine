/** Public bench scale. Set TCC_BENCH_SCALE=quick for smoke runs. */

export type BenchScale = {
  name: "full" | "quick";
  warmup: number;
  measured: number;
  recoverSamples: number;
  foreignWalSizes: number[];
  concurrencyLevels: number[];
  liveStateLocals: number[];
  historyRevisions: number[];
  recoveryReplayDepths: number[];
  recoveryReplayLiveBytes: number[];
  targetSuffix: number;
};

export function benchScale(): BenchScale {
  if (process.env.TCC_BENCH_SCALE === "quick") {
    return {
      name: "quick",
      warmup: 5,
      measured: 20,
      recoverSamples: 20,
      foreignWalSizes: [0, 100, 1_000],
      concurrencyLevels: [1, 4, 8],
      liveStateLocals: [4, 256],
      historyRevisions: [1, 8, 33],
      recoveryReplayDepths: [10, 100],
      recoveryReplayLiveBytes: [4096],
      targetSuffix: 10,
    };
  }
  return {
    name: "full",
    warmup: 100,
    measured: 1_000,
    recoverSamples: 100,
    foreignWalSizes: [0, 100, 1_000, 10_000, 100_000],
    concurrencyLevels: [1, 2, 4, 8, 16, 32],
    liveStateLocals: [4, 256, 1024],
    historyRevisions: [1, 8, 33, 64],
    recoveryReplayDepths: [10, 100, 500, 1_000, 5_000],
    recoveryReplayLiveBytes: [4096, 16_384, 65_536],
    targetSuffix: 10,
  };
}

export const DISCLAIMER =
  "These benchmarks characterize the portable TCC engine on the local host. They are not the research-prototype measurements and do not represent a hosted service.";

/** Internal A/B: TCC_BENCH_STORAGE=naive|optimized limits the persist loop. */
export function persistModes(): Array<"naive" | "optimized"> {
  const raw = process.env.TCC_BENCH_STORAGE;
  if (raw === "naive" || raw === "optimized") {
    return [raw];
  }
  return ["naive", "optimized"];
}
