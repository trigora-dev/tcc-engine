/** Semantic continuation deltas. Reconstruction is host packing, not instruction replay. */

export const MATERIALIZE_EVERY = 32;

export type PersistKind = "snapshot" | "delta";
export type PersistMode = "naive" | "optimized" | "replay";
export type PersistPacking = "follow" | "adaptive";

export type PackingThresholds = {
  minFullBytes: number;
  maxDeltaRatio: number;
};

/** Node optimized default. Full A–D/wave confirm did not beat follow; keep follow. */
export const NODE_PACKING_DEFAULT: PersistPacking = "follow";

export const NODE_ADAPTIVE_THRESHOLDS: PackingThresholds = {
  minFullBytes: 1024,
  maxDeltaRatio: 0.5,
};

export function packingFromEnv(fallback: PersistPacking = NODE_PACKING_DEFAULT): PersistPacking {
  const value = process.env.TCC_PERSIST_PACKING;
  if (value === "follow" || value === "adaptive") {
    return value;
  }
  return fallback;
}

export function packingThresholdsFromEnv(fallback: PackingThresholds = NODE_ADAPTIVE_THRESHOLDS): PackingThresholds {
  const minRaw = process.env.TCC_PERSIST_MIN_FULL_BYTES;
  const ratioRaw = process.env.TCC_PERSIST_MAX_DELTA_RATIO;
  const minFullBytes = minRaw !== undefined && minRaw !== "" ? Number(minRaw) : fallback.minFullBytes;
  const maxDeltaRatio = ratioRaw !== undefined && ratioRaw !== "" ? Number(ratioRaw) : fallback.maxDeltaRatio;
  return {
    minFullBytes: Number.isFinite(minFullBytes) ? minFullBytes : fallback.minFullBytes,
    maxDeltaRatio: Number.isFinite(maxDeltaRatio) ? maxDeltaRatio : fallback.maxDeltaRatio,
  };
}

/**
 * Host packing only. Snapshot and delta of the same revision are equivalent
 * durable representations. Never invents a delta.
 */
export function choosePackedKind(input: {
  mustMaterialize: boolean;
  packing: PersistPacking;
  fullBytes: number;
  deltaBytes: number | null;
  thresholds: PackingThresholds;
}): PersistKind {
  if (input.mustMaterialize) {
    return "snapshot";
  }
  if (input.packing === "follow") {
    return "delta";
  }
  if (input.deltaBytes === null) {
    return "snapshot";
  }
  if (input.fullBytes < input.thresholds.minFullBytes) {
    return "snapshot";
  }
  if (input.fullBytes === 0 || input.deltaBytes / input.fullBytes > input.thresholds.maxDeltaRatio) {
    return "snapshot";
  }
  return "delta";
}

export type TaggedValue = { t: string; v?: unknown };

export type ContinuationJson = {
  execution_id: string;
  artifact_hash: string;
  engine_format_version: number;
  language_semantics_version: string;
  revision: number;
  status: string;
  frames: Array<{ func_id: number; pc: number; locals: TaggedValue[] }>;
  stack: TaggedValue[];
  pending: unknown;
  result: unknown;
  try_stack: unknown[];
};

export type ContinuationDelta = {
  frames?: Array<{
    index: number;
    pc?: number;
    locals?: Array<{ slot: number; value: TaggedValue }>;
  }>;
  stack?: TaggedValue[];
  pending?: unknown;
  status?: string;
  result?: unknown;
  try_stack?: unknown[];
};

export function persistModeFromEnv(): PersistMode {
  return process.env.TCC_PERSIST === "naive" ? "naive" : "optimized";
}

export function applyDelta(base: ContinuationJson, delta: ContinuationDelta): ContinuationJson {
  const next: ContinuationJson = {
    ...base,
    frames: base.frames.map((frame) => ({
      ...frame,
      locals: frame.locals.slice(),
    })),
    stack: base.stack.slice(),
    try_stack: Array.isArray(base.try_stack) ? base.try_stack.slice() : [],
  };
  for (const frameDelta of delta.frames ?? []) {
    const frame = next.frames[frameDelta.index];
    if (!frame) {
      throw new Error(`delta frame index ${frameDelta.index} is out of range`);
    }
    if (frameDelta.pc !== undefined) {
      frame.pc = frameDelta.pc;
    }
    for (const patch of frameDelta.locals ?? []) {
      if (patch.slot < 0 || patch.slot >= frame.locals.length) {
        throw new Error(`delta local slot ${patch.slot} is out of range`);
      }
      frame.locals[patch.slot] = patch.value;
    }
  }
  if (Object.prototype.hasOwnProperty.call(delta, "stack")) {
    next.stack = delta.stack ?? [];
  }
  if (Object.prototype.hasOwnProperty.call(delta, "pending")) {
    next.pending = delta.pending ?? null;
  }
  if (delta.status !== undefined) {
    next.status = delta.status;
  }
  if (Object.prototype.hasOwnProperty.call(delta, "result")) {
    next.result = delta.result ?? null;
  }
  if (Object.prototype.hasOwnProperty.call(delta, "try_stack")) {
    next.try_stack = delta.try_stack ?? [];
  }
  return next;
}

export function reconstructContinuation(
  snapshot: ContinuationJson,
  deltas: ContinuationDelta[],
  revision: number,
): ContinuationJson {
  let current = snapshot;
  for (const delta of deltas) {
    current = applyDelta(current, delta);
  }
  current.revision = revision;
  return current;
}
