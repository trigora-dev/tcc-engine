/** Semantic continuation deltas. Reconstruction is host packing, not instruction replay. */

export const MATERIALIZE_EVERY = 32;

export type PersistKind = "snapshot" | "delta";
export type PersistMode = "naive" | "optimized";

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
