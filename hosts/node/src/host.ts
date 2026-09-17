import { appendFileSync } from "node:fs";

import { EngineBinding, loadEngine, type Outcome } from "../../../bindings/js/src/index.ts";
import { maybeCrash } from "./crash.ts";
import { Store } from "./store.ts";

export type FakeEffects = Record<string, unknown>;

export type RunOptions = {
  dbPath: string;
  wasmPath: string;
  artifactJson: string;
  executionId?: string;
  effects?: FakeEffects;
  eventPayload?: unknown;
  ownerToken?: string;
  leaseMs?: number;
  budget?: number;
  autoDeliverEvent?: boolean;
  effectLogPath?: string;
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

export async function startExecution(options: RunOptions): Promise<RunResult> {
  await loadEngine(options.wasmPath);
  const store = new Store(options.dbPath);
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
  const store = new Store(options.dbPath);
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
    if (execution.status === "completed" && saved) {
      const parsed = JSON.parse(saved.json) as { result: unknown };
      return {
        status: "completed",
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
  const effects = options.effects ?? { generate: 42 };
  const budget = options.budget ?? 256;

  for (;;) {
    const outcome = engine.runUntilHost(budget);
    const next = handleOutcome(store, engine, outcome, effects, options);
    if (next === "continue") {
      continue;
    }
    return next;
  }
}

function handleOutcome(
  store: Store,
  engine: EngineBinding,
  outcome: Outcome,
  effects: FakeEffects,
  options: RunOptions & { ownerToken: string; executionId: string },
): "continue" | RunResult {
  switch (outcome.type) {
    case "completed":
      return snapshot(store, options.executionId, "completed", outcome.result);
    case "failed":
      throw new Error(outcome.message);
    case "budget_exhausted":
      throw new Error("instruction budget exhausted");
    case "suspended":
      return deliverEvent(store, engine, options);
    case "host": {
      const request = outcome.request;
      switch (request.type) {
        case "run_effect":
          return runEffect(store, engine, request, effects, options);
        case "persist_effect":
          return persistEffect(store, engine, request, options.executionId);
        case "register_wait":
          return registerWait(store, engine, request, options.executionId);
        case "persist_checkpoint":
          return persistCheckpoint(store, engine, request, options);
        case "register_timer":
        case "create_child":
          engine.applyHostResponse({ type: "ack" });
          return "continue";
        default:
          throw new Error(`unsupported host request \`${String(request.type)}\``);
      }
    }
    default:
      throw new Error("unknown engine outcome");
  }
}

function runEffect(
  store: Store,
  engine: EngineBinding,
  request: Record<string, unknown>,
  effects: FakeEffects,
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
  if (!(key in effects)) {
    throw new Error(`no fake effect for \`${key}\``);
  }
  if (options.effectLogPath) {
    appendFileSync(options.effectLogPath, `${key}\n`);
  }
  const value = encodeValue(effects[key]);
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
  store.completeEffect(executionId, key, idempotencyKey, resultJson);
  maybeCrash("after_persist_effect");
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
  store.commitCheckpoint(options.executionId, revision, json, parsed.status, options.ownerToken);
  maybeCrash("after_persist_checkpoint");
  if (parsed.status === "suspended") {
    maybeCrash("after_wait_checkpoint");
  }
  engine.applyHostResponse({ type: "persist_confirmed", revision });
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
  if (value && typeof value === "object" && !Array.isArray(value)) {
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
