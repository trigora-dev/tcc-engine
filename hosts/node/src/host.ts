import { appendFileSync } from "node:fs";

import { EngineBinding, loadEngine, type Outcome } from "@tcc-engine/bindings-javascript";
import { maybeCrash } from "./crash.ts";
import { Store, type PersistMode } from "./store.ts";

export type FakeEffects = Record<string, unknown>;
export type EffectRunner = (key: string) => unknown;

export type RunOptions = {
  dbPath: string;
  wasmPath?: string;
  artifactJson: string;
  executionId?: string;
  /** Product callback. The CLI/SDK supplies effect results by key. */
  runEffect?: EffectRunner;
  /** Test convenience map. Ignored when `runEffect` is set. */
  effects?: FakeEffects;
  eventPayload?: unknown;
  ownerToken?: string;
  leaseMs?: number;
  budget?: number;
  autoDeliverEvent?: boolean;
  effectLogPath?: string;
  failCounts?: Record<string, number>;
  childArtifacts?: Record<string, string>;
  cancel?: boolean;
  persist?: PersistMode;
};

export type ResumeOptions = Omit<RunOptions, "artifactJson"> & {
  artifactJson?: string;
};

export type RunResult = {
  status: string;
  result: unknown;
  continuationJson: string | null;
  revision: number;
};

export function mapEffects(effects: FakeEffects): EffectRunner {
  return (key) => {
    if (!(key in effects)) {
      throw new Error(`no effect for \`${key}\``);
    }
    return effects[key];
  };
}

function effectRunner(options: RunOptions): EffectRunner {
  if (options.runEffect) {
    return options.runEffect;
  }
  return mapEffects(options.effects ?? { generate: 42 });
}

export async function startExecution(options: RunOptions): Promise<RunResult> {
  await loadEngine(options.wasmPath);
  const store = new Store(options.dbPath, options.persist);
  try {
    const executionId = options.executionId ?? "first";
    const ownerToken = options.ownerToken ?? "owner-1";
    const envelope = JSON.parse(options.artifactJson) as { envelope: { artifact_hash: string } };
    const hash = envelope.envelope.artifact_hash;
    store.putArtifact(hash, options.artifactJson);
    store.createExecution(executionId, hash, ownerToken, Date.now() + (options.leaseMs ?? 60_000));
    const engine = new EngineBinding(options.artifactJson, executionId);
    return drive(store, engine, {
      ...options,
      executionId,
      ownerToken,
      autoDeliverEvent: options.autoDeliverEvent ?? true,
    });
  } finally {
    store.close();
  }
}

export async function resumeExecution(options: ResumeOptions): Promise<RunResult> {
  await loadEngine(options.wasmPath);
  const store = new Store(options.dbPath, options.persist);
  try {
    const executionId = options.executionId ?? "first";
    const ownerToken = options.ownerToken ?? "owner-1";
    store.takeLease(executionId, ownerToken, Date.now() + (options.leaseMs ?? 60_000), Date.now());
    const execution = store.getExecution(executionId);
    if (!execution) {
      throw new Error(`unknown execution \`${executionId}\``);
    }
    const saved = store.getContinuation(executionId);
    const artifactJson = options.artifactJson ?? store.getArtifact(execution.artifact_hash);
    if (!artifactJson) {
      throw new Error(`missing artifact \`${execution.artifact_hash}\``);
    }
    if ((execution.status === "completed" || execution.status === "cancelled" || execution.status === "failed") && saved) {
      const parsed = JSON.parse(saved.json) as { result: unknown };
      return {
        status: execution.status,
        result: parsed.result,
        continuationJson: saved.json,
        revision: saved.revision,
      };
    }
    const engine = saved
      ? EngineBinding.resume(artifactJson, saved.json)
      : new EngineBinding(artifactJson, executionId);
    return drive(store, engine, {
      wasmPath: options.wasmPath,
      dbPath: options.dbPath,
      artifactJson,
      executionId,
      ownerToken,
      effects: options.effects,
      eventPayload: options.eventPayload,
      budget: options.budget,
      autoDeliverEvent: options.autoDeliverEvent ?? true,
      effectLogPath: options.effectLogPath,
      leaseMs: options.leaseMs,
      failCounts: options.failCounts,
      childArtifacts: options.childArtifacts,
      cancel: options.cancel,
    });
  } finally {
    store.close();
  }
}

function drive(
  store: Store,
  engine: EngineBinding,
  options: RunOptions & { ownerToken: string; executionId: string },
): RunResult {
  const runEffect = effectRunner(options);
  const budget = options.budget ?? 256;
  const state = { engine };
  const failCounts = { ...(options.failCounts ?? {}) };

  for (;;) {
    const outcome = state.engine.runUntilHost(budget);
    const next = handleOutcome(store, state, outcome, runEffect, failCounts, options);
    if (next === "continue") {
      continue;
    }
    return next;
  }
}

type EngineState = { engine: EngineBinding };

function handleOutcome(
  store: Store,
  state: EngineState,
  outcome: Outcome,
  runEffect: EffectRunner,
  failCounts: Record<string, number>,
  options: RunOptions & { ownerToken: string; executionId: string },
): "continue" | RunResult {
  switch (outcome.type) {
    case "completed":
      return snapshot(store, options.executionId, "completed", outcome.result);
    case "failed":
      return snapshot(store, options.executionId, "failed", outcome.message);
    case "cancelled":
      return snapshot(store, options.executionId, "cancelled", undefined);
    case "budget_exhausted":
      throw new Error("instruction budget exhausted");
    case "suspended":
      return deliverWake(store, state, options);
    case "host": {
      const request = outcome.request;
      switch (request.type) {
        case "run_effect":
          return executeEffect(store, state.engine, request, runEffect, failCounts, options);
        case "persist_effect":
          return persistEffect(store, state.engine, request, options.executionId);
        case "register_wait":
          return registerWait(store, state.engine, request, options.executionId);
        case "persist_checkpoint":
          return persistCheckpoint(store, state.engine, request, options);
        case "register_timer":
          store.upsertTimer(options.executionId, Number(request.resume_at_ms ?? 0));
          maybeCrash("after_register_timer");
          state.engine.applyHostResponse({ type: "ack" });
          return "continue";
        case "create_child":
          store.upsertChild(
            String(request.invoke_id),
            options.executionId,
            String(request.child_execution_id),
            String(request.flow_name),
          );
          maybeCrash("after_create_child");
          state.engine.applyHostResponse({ type: "ack" });
          return "continue";
        default:
          throw new Error(`unsupported host request \`${String(request.type)}\``);
      }
    }
    default:
      throw new Error("unknown engine outcome");
  }
}

function executeEffect(
  store: Store,
  engine: EngineBinding,
  request: Record<string, unknown>,
  runEffect: EffectRunner,
  failCounts: Record<string, number>,
  options: RunOptions & { executionId: string },
): "continue" {
  const key = String(request.key);
  const idempotencyKey = String(request.idempotency_key);
  maybeCrash("before_effect_provider");
  const existing = store.getEffect(options.executionId, key);
  if (existing?.status === "completed" && existing.result_json) {
    engine.applyHostResponse({
      type: "effect_result",
      value: JSON.parse(existing.result_json),
    });
    return "continue";
  }
  store.markEffectStarted(options.executionId, key, idempotencyKey);
  if ((failCounts[key] ?? 0) > 0) {
    failCounts[key] = (failCounts[key] ?? 0) - 1;
    if (options.effectLogPath) {
      appendFileSync(options.effectLogPath, `${key}\n`);
    }
    store.failEffect(options.executionId, key, idempotencyKey, JSON.stringify({ t: "string", v: "failed" }));
    maybeCrash("after_persist_effect", key);
    return executeEffect(store, engine, request, runEffect, failCounts, options);
  }
  if (options.effectLogPath) {
    appendFileSync(options.effectLogPath, `${key}\n`);
  }
  const value = encodeValue(runEffect(key));
  maybeCrash("after_effect_provider");
  engine.applyHostResponse({ type: "effect_result", value });
  return "continue";
}

function persistEffect(
  store: Store,
  engine: EngineBinding,
  request: Record<string, unknown>,
  executionId: string,
): "continue" {
  maybeCrash("before_persist_effect");
  const key = String(request.key);
  const idempotencyKey = String(request.idempotency_key ?? `${executionId}:${key}`);
  const resultJson = JSON.stringify(request.result ?? null);
  if (request.status === "failed") {
    store.failEffect(executionId, key, idempotencyKey, resultJson);
  } else {
    store.completeEffect(executionId, key, idempotencyKey, resultJson);
  }
  maybeCrash("after_persist_effect", key);
  engine.applyHostResponse({ type: "ack" });
  return "continue";
}

function registerWait(
  store: Store,
  engine: EngineBinding,
  request: Record<string, unknown>,
  executionId: string,
): "continue" {
  store.upsertWait(String(request.wait_id), executionId, String(request.event_name));
  maybeCrash("after_register_wait");
  engine.applyHostResponse({ type: "ack" });
  return "continue";
}

function persistCheckpoint(
  store: Store,
  engine: EngineBinding,
  request: Record<string, unknown>,
  options: RunOptions & { ownerToken: string; executionId: string },
): "continue" {
  maybeCrash("before_persist_checkpoint");
  const revision = Number(request.revision);
  const parsed = JSON.parse(engine.continuationJson()) as {
    status: string;
    revision: number;
  };
  parsed.revision = revision;
  const json = JSON.stringify(parsed);
  const kind = request.kind === "delta" ? "delta" : "snapshot";
  store.commitCheckpoint(options.executionId, revision, json, parsed.status, options.ownerToken, {
    kind,
    materialize: request.materialize === true || kind === "snapshot",
    deltaJson: request.delta == null ? null : JSON.stringify(request.delta),
  });
  maybeCrash("after_persist_checkpoint");
  if (parsed.status === "suspended") {
    maybeCrash("after_wait_checkpoint");
  }
  engine.applyHostResponse({ type: "persist_confirmed", revision });
  return "continue";
}

function deliverWake(
  store: Store,
  state: EngineState,
  options: RunOptions & { ownerToken: string; executionId: string },
): "continue" | RunResult {
  if (options.cancel) {
    maybeCrash("before_cancel");
    state.engine.applyHostResponse({ type: "cancel" });
    return "continue";
  }
  const continuation = JSON.parse(state.engine.continuationJson()) as {
    pending?: { kind?: { type?: string; invoke_id?: string } };
  };
  const waitKind = continuation.pending?.kind?.type;
  if (waitKind === "timer") {
    const timer = store.pendingTimer(options.executionId);
    if (!timer || timer.status !== "pending") {
      return snapshot(store, options.executionId, "suspended", undefined);
    }
    maybeCrash("before_timer_fired");
    store.resolveTimer(options.executionId);
    state.engine.applyHostResponse({ type: "timer_fired" });
    return "continue";
  }
  if (waitKind === "child") {
    return deliverChild(store, state, options, continuation.pending?.kind?.invoke_id);
  }
  return deliverEvent(store, state.engine, options);
}

function deliverChild(
  store: Store,
  state: EngineState,
  options: RunOptions & { ownerToken: string; executionId: string },
  invokeId: string | undefined,
): "continue" | RunResult {
  if (!invokeId) {
    return snapshot(store, options.executionId, "suspended", undefined);
  }
  const child = store.getChild(invokeId);
  if (!child) {
    return snapshot(store, options.executionId, "suspended", undefined);
  }
  if (child.status === "completed" && child.result_json) {
    maybeCrash("before_child_result");
    state.engine.applyHostResponse({
      type: "child_result",
      value: JSON.parse(child.result_json),
    });
    return "continue";
  }
  const artifactJson = options.childArtifacts?.[child.flow_name];
  if (!artifactJson) {
    throw new Error(`no child artifact for \`${child.flow_name}\``);
  }
  const parentJson = state.engine.continuationJson();
  const parentArtifact = options.artifactJson;
  store.putArtifact(JSON.parse(artifactJson).envelope.artifact_hash, artifactJson);
  const existing = store.getExecution(child.child_execution_id);
  if (!existing) {
    store.createExecution(
      child.child_execution_id,
      JSON.parse(artifactJson).envelope.artifact_hash,
      options.ownerToken,
      Date.now() + (options.leaseMs ?? 60_000),
    );
  }
  const childEngine = existing
    ? (() => {
        const saved = store.getContinuation(child.child_execution_id);
        return saved
          ? EngineBinding.resume(artifactJson, saved.json)
          : new EngineBinding(artifactJson, child.child_execution_id);
      })()
    : new EngineBinding(artifactJson, child.child_execution_id);
  const childResult = drive(store, childEngine, {
    ...options,
    executionId: child.child_execution_id,
    artifactJson,
    cancel: false,
    childArtifacts: options.childArtifacts,
    autoDeliverEvent: options.autoDeliverEvent,
  });
  store.completeChild(invokeId, JSON.stringify(childResult.result ?? { t: "undefined" }));
  state.engine = EngineBinding.resume(parentArtifact, parentJson);
  maybeCrash("before_child_result");
  state.engine.applyHostResponse({
    type: "child_result",
    value: childResult.result,
  });
  return "continue";
}

function deliverEvent(
  store: Store,
  engine: EngineBinding,
  options: RunOptions & { executionId: string },
): "continue" | RunResult {
  const wait = store.pendingWait(options.executionId);
  if (!wait) {
    return snapshot(store, options.executionId, "suspended", undefined);
  }
  if (wait.status !== "pending") {
    return snapshot(store, options.executionId, "suspended", undefined);
  }
  maybeCrash("before_event_payload");
  let payloadJson: string | undefined;
  const queued = store.takeEvent(options.executionId, wait.event_name);
  if (queued) {
    payloadJson = queued.payload_json;
  } else if (options.autoDeliverEvent !== false && options.eventPayload !== undefined) {
    payloadJson = JSON.stringify(encodeValue(options.eventPayload));
  } else if (options.autoDeliverEvent !== false && options.eventPayload === undefined) {
    payloadJson = JSON.stringify(encodeValue("ok"));
  }
  if (!payloadJson) {
    return snapshot(store, options.executionId, "suspended", undefined);
  }
  store.resolveWait(wait.wait_id);
  maybeCrash("after_event_persist");
  engine.applyHostResponse({
    type: "event_payload",
    value: JSON.parse(payloadJson),
  });
  return "continue";
}

function snapshot(
  store: Store,
  executionId: string,
  status: string,
  result: unknown,
): RunResult {
  const saved = store.getContinuation(executionId);
  return {
    status,
    result,
    continuationJson: saved?.json ?? null,
    revision: saved?.revision ?? 0,
  };
}

export function encodeValue(value: unknown): { t: string; v?: unknown } {
  if (value === undefined) {
    return { t: "undefined" };
  }
  if (value === null) {
    return { t: "null" };
  }
  if (typeof value === "boolean") {
    return { t: "bool", v: value };
  }
  if (typeof value === "number") {
    return { t: "number", v: value };
  }
  if (typeof value === "string") {
    return { t: "string", v: value };
  }
  if (Array.isArray(value)) {
    return { t: "array", v: value.map(encodeValue) };
  }
  if (value && typeof value === "object") {
    const fields: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) {
      fields[key] = encodeValue(item);
    }
    return { t: "object", v: fields };
  }
  throw new Error("unsupported fake value");
}

export function deliverDuplicateEvent(dbPath: string, executionId: string, name: string, payload: unknown): void {
  const store = new Store(dbPath);
  try {
    const wait = store.pendingWait(executionId);
    if (!wait || wait.status !== "pending") {
      return;
    }
    store.enqueueEvent(executionId, name, JSON.stringify(encodeValue(payload)));
  } finally {
    store.close();
  }
}
