export type LatencyStats = {
  meanMs: number;
  medianMs: number;
  p95Ms: number;
  p99Ms: number;
  minMs: number;
  maxMs: number;
  samples: number;
};

export function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) {
    return 0;
  }
  const idx = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
  return sorted[idx]!;
}

export function latencyStats(samplesMs: number[]): LatencyStats {
  const sorted = samplesMs.slice().sort((a, b) => a - b);
  const sum = sorted.reduce((acc, value) => acc + value, 0);
  return {
    meanMs: Number((sum / Math.max(1, sorted.length)).toFixed(4)),
    medianMs: Number(percentile(sorted, 50).toFixed(4)),
    p95Ms: Number(percentile(sorted, 95).toFixed(4)),
    p99Ms: Number(percentile(sorted, 99).toFixed(4)),
    minMs: Number((sorted[0] ?? 0).toFixed(4)),
    maxMs: Number((sorted[sorted.length - 1] ?? 0).toFixed(4)),
    samples: sorted.length,
  };
}

export function throughputPerSec(ops: number, totalMs: number): number {
  if (totalMs <= 0) {
    return 0;
  }
  return Number((ops / (totalMs / 1000)).toFixed(2));
}

/** Fraction slower than baseline from robust latency. Positive = slower. */
export function relativeOverhead(candidateMs: number, baselineMs: number): number | null {
  if (!(baselineMs > 0) || !Number.isFinite(candidateMs)) {
    return null;
  }
  return Number(((candidateMs - baselineMs) / baselineMs).toFixed(4));
}

export function summarizeOccupancy(samples: number[]): {
  avg: number;
  max: number;
  count: number;
} {
  if (samples.length === 0) {
    return { avg: 0, max: 0, count: 0 };
  }
  const sum = samples.reduce((acc, value) => acc + value, 0);
  return {
    avg: Number((sum / samples.length).toFixed(2)),
    max: Math.max(...samples),
    count: samples.length,
  };
}

export async function timedLoop(
  warmup: number,
  measured: number,
  body: (i: number) => void | Promise<void>,
  afterWarmup?: () => void,
): Promise<{ samplesMs: number[]; totalMeasuredMs: number }> {
  for (let i = 0; i < warmup; i++) {
    await body(i);
  }
  afterWarmup?.();
  const samplesMs: number[] = [];
  const started = performance.now();
  for (let i = 0; i < measured; i++) {
    const t0 = performance.now();
    await body(warmup + i);
    samplesMs.push(performance.now() - t0);
  }
  return { samplesMs, totalMeasuredMs: performance.now() - started };
}
