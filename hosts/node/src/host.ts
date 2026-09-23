import { appendFileSync } from "node:fs";

import { EngineBinding, loadEngine, type Outcome } from "@tcc-engine/bindings-javascript";
import { maybeCrash } from "./crash.ts";
import { Store, type PackingThresholds, type PersistMode, type PersistPacking } from "./store.ts";
import type { HostObserver } from "./observe.ts";

export type FakeEffects = Record<string, unknown>;
export type EffectRunner = (key: string, input: unknown) => unknown;

export async function ensureEngineLoaded(wasmPath?: string): Promise<void> {
  await loadEngine(wasmPath);
}

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
  /** Deliver one registered join branch per wake. `reverse` completes the highest index first. */
  completionOrder?: "source" | "reverse";
  effectLogPath?: string;
  failCounts?: Record<string, number>;
  childArtifacts?: Record<string, string>;
  /** Ordered program arguments. Absent or `[]` means no arguments. */
  args?: unknown[];
  /** Already-encoded argument-array JSON for a child row. Takes precedence over `args`. */
  programArgsJson?: string;
  cancel?: boolean;
  persist?: PersistMode;
  packing?: PersistPacking;
  packingThresholds?: PackingThresholds;
  /** Optional host-agnostic observability sink. Must not throw into persist. */
  onEvent?: HostObserver;
  /** Give this execution its own WASM instance when several engines are in flight. */
  isolatedEngine?: boolean;
  profile?: PersistProfile;
};

export type PersistProfile = {
  engineRequestMs: number;
  continuationEncodeMs: number;
  hostEncodeMs: number;
  storeCommitMs: number;
  persistConfirmedMs: number;
  checkpoints: number;
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
  return (key, _input) => {
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

function storeFromOptions(path: string, options: RunOptions | ResumeOptions): Store {
  return new Store(path, options.persist, options.onEvent, {
    packing: options.packing,
    packingThresholds: options.packingThresholds,
  });
}

export async function startExecution(options: RunOptions): Promise<RunResult> {
  await loadEngine(options.wasmPath);
  const store = storeFromOptions(options.dbPath, options);
  try {
    return runOnStore(store, options);
  } finally {
    store.close();
  }
}

/** Drive one execution on an existing store (shared-DB / group-commit benches). */
export function runOnStore(
  store: Store,
  options: RunOptions & { artifactJson: string },
): RunResult {
  if (store.isGrouping()) {
    throw new Error("runOnStore cannot acknowledge checkpoints inside an open group; use runBatchOnStore");
  }
  const prepared = prepareOnStore(store, options);
  return drive(store, prepared.engine, prepared.options);
}

function startArgsJson(options: RunOptions): string | undefined {
  if (options.programArgsJson !== undefined) {
    return options.programArgsJson === "[]" ? undefined : options.programArgsJson;
  }
  if (!Object.prototype.hasOwnProperty.call(options, "args")) {
    return undefined;
  }
  if (!Array.isArray(options.args)) {
    throw new Error("program args must be an array");
  }
  if (options.args.length === 0) {
    return undefined;
  }
  return JSON.stringify(options.args.map((item) => encodeValue(item)));
}

function prepareOnStore(store: Store, options: RunOptions & { artifactJson: string }) {
  const executionId = options.executionId ?? "first";
  const ownerToken = options.ownerToken ?? "owner-1";
  const envelope = JSON.parse(options.artifactJson) as { envelope: { artifact_hash: string } };
  const hash = envelope.envelope.artifact_hash;
  store.putArtifact(hash, options.artifactJson);
  if (!store.getExecution(executionId)) {
    store.createExecution(executionId, hash, ownerToken, Date.now() + (options.leaseMs ?? 60_000));
  }
  const engine = new EngineBinding(
    options.artifactJson,
    executionId,
    options.isolatedEngine === true,
    startArgsJson(options),
  );
  return {
    engine,
    options: {
      ...options,
      executionId,
      ownerToken,
      autoDeliverEvent: options.autoDeliverEvent ?? true,
      failCounts: { ...(options.failCounts ?? {}) },
    },
  };
}

/** Cooperatively advance executions, committing each durability batch before acknowledgment. */
export function runBatchOnStore(
  store: Store,
  inputs: Array<RunOptions & { artifactJson: string }>,
  beforeCommit?: () => void,
  afterCommit?: () => void,
): RunResult[] {
  if (store.isGrouping()) {
    throw new Error("batch runner requires a store without an open group");
  }
  const sessions = inputs.map((input) => prepareOnStore(store, { ...input, isolatedEngine: true }));
  const results: Array<RunResult | undefined> = Array(inputs.length).fill(undefined);
  while (results.some((result) => result === undefined)) {
    const queued: Array<{ index: number; revision: number }> = [];
    store.beginGroup();
    try {
      for (let i = 0; i < sessions.length; i++) {
        if (results[i]) continue;
        const session = sessions[i]!;
        const result = drive(store, session.engine, session.options, true);
        if ("queuedRevision" in result) queued.push({ index: i, revision: result.queuedRevision });
        else if (!("activateChild" in result)) results[i] = result;
      }
      if (queued.length > 0) beforeCommit?.();
      store.endGroup();
    } catch (error) {
      store.abortGroup();
      throw error;
    }
    if (queued.length > 0) afterCommit?.();
    for (const item of queued) {
      sessions[item.index]!.engine.applyHostResponse({ type: "persist_confirmed", revision: item.revision });
    }
    for (const created of store.takeCreatedChildren()) {
      if (sessions.some((session) => session.options.executionId === created.childExecutionId)) {
        continue;
      }
      const parent = sessions.find((session) => session.options.executionId === created.parentExecutionId);
      const artifactJson = parent?.options.childArtifacts?.[created.flowName];
      if (!artifactJson) {
        throw new Error(`no child artifact for \`${created.flowName}\``);
      }
      const { args: _parentArgs, programArgsJson: _parentArgsJson, ...rest } = parent!.options;
      const row = store.getChildByExecutionId(created.childExecutionId);
      sessions.push(
        prepareOnStore(store, {
          ...rest,
          executionId: created.childExecutionId,
          artifactJson,
          isolatedEngine: true,
          cancel: false,
          ...(row?.args_json != null ? { programArgsJson: row.args_json } : {}),
        }),
      );
      results.push(undefined);
    }
  }
  return results.slice(0, inputs.length) as RunResult[];
}

export async function resumeExecution(options: ResumeOptions): Promise<RunResult> {
  await loadEngine(options.wasmPath);
  const store = storeFromOptions(options.dbPath, options);
  try {
    const executionId = options.executionId ?? "first";
    const ownerToken = options.ownerToken ?? "owner-1";
    store.takeLease(executionId, ownerToken, Date.now() + (options.leaseMs ?? 60_000), Date.now());
    const execution = store.getExecution(executionId);
    if (!execution) {
      throw new Error(`unknown execution \`${executionId}\``);
    }
    const saved = store.getContinuation(executionId, { observeRestore: true });
    const artifactJson = options.artifactJson ?? store.getArtifact(execution.artifact_hash);
    if (!artifactJson) {
      throw new Error(`missing artifact \`${execution.artifact_hash}\``);
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
      completionOrder: options.completionOrder,
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
): RunResult;
function drive(
  store: Store,
  engine: EngineBinding,
  options: RunOptions & { ownerToken: string; executionId: string },
  deferCheckpoint: true,
): RunResult | { queuedRevision: number } | { activateChild: true };
function drive(
  store: Store,
  engine: EngineBinding,
  options: RunOptions & { ownerToken: string; executionId: string },
  deferCheckpoint = false,
): RunResult | { queuedRevision: number } | { activateChild: true } {
  if (store.isGrouping() && !deferCheckpoint) {
    throw new Error("single-execution drive cannot run inside an open checkpoint group");
  }
  const runEffect = effectRunner(options);
  const budget = options.budget ?? 256;
  const state = { engine };
  const failCounts = options.failCounts ?? {};

  for (;;) {
    const requestStart = options.profile ? performance.now() : 0;
    const outcome = state.engine.runUntilHost(budget);
    if (options.profile) options.profile.engineRequestMs += performance.now() - requestStart;
    const next = handleOutcome(store, state, outcome, runEffect, failCounts, options, deferCheckpoint);
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
  deferCheckpoint = false,
): "continue" | { queuedRevision: number } | { activateChild: true } | RunResult {
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
      return deliverWake(store, state, options, deferCheckpoint);
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
          return persistCheckpoint(store, state.engine, request, options, deferCheckpoint);
        case "register_timer":
          store.upsertTimer(
            options.executionId,
            Number(request.resume_at_ms ?? 0),
            typeof request.branch === "string" ? request.branch : "",
          );
          maybeCrash("after_register_timer");
          state.engine.applyHostResponse({ type: "ack" });
          return "continue";
        case "create_child":
          return enqueueChild(store, state.engine, request, options);
        default:
          store.observe({ type: "runtime.error", message: `unsupported host request \`${String(request.type)}\`` });
          throw new Error(`unsupported host request \`${String(request.type)}\``);
      }
    }
    default:
      store.observe({ type: "runtime.error", message: "unknown engine outcome" });
      throw new Error("unknown engine outcome");
  }
}

function enqueueChild(
  store: Store,
  engine: EngineBinding,
  request: Record<string, unknown>,
  options: RunOptions & { ownerToken: string; executionId: string },
): "continue" {
  const programName = String(request.program_name);
  const artifactJson = options.childArtifacts?.[programName];
  if (!artifactJson) {
    store.observe({ type: "runtime.error", executionId: options.executionId, message: `no child artifact for \`${programName}\`` });
    throw new Error(`no child artifact for \`${programName}\``);
  }
  const artifactHash = String(JSON.parse(artifactJson).envelope.artifact_hash);
  store.putArtifact(artifactHash, artifactJson);
  store.enqueueCreateChild({
    invokeId: String(request.invoke_id),
    parentExecutionId: options.executionId,
    childExecutionId: String(request.child_execution_id),
    flowName: programName,
    artifactHash,
    ownerToken: options.ownerToken,
    leaseUntil: Date.now() + (options.leaseMs ?? 60_000),
    argsJson: childArgsJson(request),
  });
  maybeCrash("after_create_child");
  engine.applyHostResponse({ type: "ack" });
  return "continue";
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
  const input = requireEffectInput(request);
  const inputJson = canonicalEffectInput(input);
  maybeCrash("before_effect_provider");
  const existing = store.getEffect(options.executionId, key);
  if (existing && effectInputJson(existing.input_json) !== inputJson) {
    throw new Error(`effect ${options.executionId}:${key} input mismatch`);
  }
  if (existing?.status === "completed" && existing.result_json) {
    store.observe({ type: "effect.journal_hit", executionId: options.executionId });
    engine.applyHostResponse({
      type: "effect_result",
      value: JSON.parse(existing.result_json),
    });
    return "continue";
  }
  store.markEffectStarted(options.executionId, key, idempotencyKey, inputJson);
  if ((failCounts[key] ?? 0) > 0) {
    failCounts[key] = (failCounts[key] ?? 0) - 1;
    if (options.effectLogPath) {
      appendFileSync(options.effectLogPath, `${key}\n`);
    }
    store.failEffect(
      options.executionId,
      key,
      idempotencyKey,
      JSON.stringify({ t: "string", v: "failed" }),
      inputJson,
    );
    maybeCrash("after_persist_effect", key);
    return executeEffect(store, engine, request, runEffect, failCounts, options);
  }
  if (options.effectLogPath) {
    appendFileSync(options.effectLogPath, `${key}\n`);
  }
  const produced = runEffect(key, plainEffectInput(input));
  if (produced === "__fail__") {
    engine.applyHostResponse({ type: "effect_failed", message: "failed" });
    return "continue";
  }
  const value = encodeValue(produced);
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
  const inputJson = effectInputJson(store.getEffect(executionId, key)?.input_json ?? null);
  if (request.status === "failed") {
    store.failEffect(executionId, key, idempotencyKey, resultJson, inputJson);
  } else {
    store.completeEffect(executionId, key, idempotencyKey, resultJson, inputJson);
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
  deferCheckpoint = false,
): "continue" | { queuedRevision: number } {
  maybeCrash("before_persist_checkpoint");
  const revision = Number(request.revision);
  if (store.persist === "replay") {
    const current = store.getExecution(options.executionId);
    if (current && current.revision >= revision) {
      engine.applyHostResponse({ type: "persist_confirmed", revision });
      return "continue";
    }
  }
  const continuationStart = options.profile ? performance.now() : 0;
  const parsed = JSON.parse(engine.continuationJson()) as {
    status: string;
    revision: number;
    result?: unknown;
  };
  if (options.profile) options.profile.continuationEncodeMs += performance.now() - continuationStart;
  const encodeStart = options.profile ? performance.now() : 0;
  parsed.revision = revision;
  const json = JSON.stringify(parsed);
  const kind = request.kind === "delta" ? "delta" : "snapshot";
  const deltaJson = request.delta == null ? null : JSON.stringify(request.delta);
  if (options.profile) options.profile.hostEncodeMs += performance.now() - encodeStart;
  const storeStart = options.profile ? performance.now() : 0;
  store.enqueueCheckpoint(options.executionId, revision, json, parsed.status, options.ownerToken, {
    kind,
    materialize: request.materialize === true || kind === "snapshot",
    deltaJson,
  });
  if (parsed.status === "completed") {
    const related = store.getChildByExecutionId(options.executionId);
    if (related) {
      store.enqueueCompleteChild({
        invokeId: related.invoke_id,
        resultJson: JSON.stringify(parsed.result ?? { t: "undefined" }),
      });
    }
  }
  if (deferCheckpoint) return { queuedRevision: revision };
  store.flushIfUngrouped();
  if (options.profile) {
    options.profile.storeCommitMs += performance.now() - storeStart;
    options.profile.checkpoints += 1;
  }
  maybeCrash("after_persist_checkpoint");
  if (parsed.status === "suspended") {
    maybeCrash("after_wait_checkpoint");
  }
  const confirmStart = options.profile ? performance.now() : 0;
  engine.applyHostResponse({ type: "persist_confirmed", revision });
  if (options.profile) options.profile.persistConfirmedMs += performance.now() - confirmStart;
  return "continue";
}

function deliverWake(
  store: Store,
  state: EngineState,
  options: RunOptions & { ownerToken: string; executionId: string },
  deferCheckpoint = false,
): "continue" | { activateChild: true } | RunResult {
  if (options.cancel) {
    maybeCrash("before_cancel");
    state.engine.applyHostResponse({ type: "cancel" });
    return "continue";
  }
  const continuation = JSON.parse(state.engine.continuationJson()) as {
    pending?: { kind?: { type?: string; invoke_id?: string } } | null;
    join?: {
      state?: string;
      branches?: Array<{ index: number; branch_id: string; phase: string; op?: { type?: string } | null }>;
    } | null;
  };
  if (continuation.join?.state === "active" && !continuation.pending) {
    return deliverJoin(store, state, options, continuation.join, deferCheckpoint);
  }
  const waitKind = continuation.pending?.kind?.type;
  if (waitKind === "timer") {
    const timer = store.pendingTimer(options.executionId);
    if (!timer || timer.status !== "pending") {
      return snapshot(store, options.executionId, "suspended", undefined);
    }
    maybeCrash("before_timer_fired");
    store.resolveTimer(options.executionId, timer.branch_id ?? "");
    state.engine.applyHostResponse(
      timer.branch_id ? { type: "timer_fired", branch: timer.branch_id } : { type: "timer_fired" },
    );
    return "continue";
  }
  if (waitKind === "child") {
    return deliverChild(store, state, options, continuation.pending?.kind?.invoke_id, deferCheckpoint);
  }
  return deliverEvent(store, state.engine, options);
}

function deliverJoin(
  store: Store,
  state: EngineState,
  options: RunOptions & { ownerToken: string; executionId: string },
  join: {
    branches?: Array<{ index: number; branch_id: string; phase: string; op?: { type?: string } | null }>;
  },
  deferCheckpoint = false,
): "continue" | { activateChild: true } | RunResult {
  const branches = [...(join.branches ?? [])].filter((branch) => branch.phase === "registered");
  branches.sort((left, right) => left.index - right.index);
  if (options.completionOrder === "reverse") {
    branches.reverse();
  }
  const branch = branches[0];
  if (!branch) {
    return snapshot(store, options.executionId, "suspended", undefined);
  }
  if (branch.op?.type === "timer") {
    const timer = store.pendingTimer(options.executionId, branch.branch_id);
    if (!timer || timer.status !== "pending") {
      return snapshot(store, options.executionId, "suspended", undefined);
    }
    maybeCrash("before_timer_fired");
    store.resolveTimer(options.executionId, branch.branch_id);
    state.engine.applyHostResponse({ type: "timer_fired", branch: branch.branch_id });
    return "continue";
  }
  if (branch.op?.type === "child") {
    return deliverChild(store, state, options, branch.branch_id, deferCheckpoint, branch.branch_id);
  }
  return deliverEvent(store, state.engine, options, branch.branch_id);
}

function childArgsJson(request: Record<string, unknown>): string | null {
  if (!Object.prototype.hasOwnProperty.call(request, "args")) return null;
  if (!Array.isArray(request.args)) {
    throw new Error("create_child args must be an array");
  }
  if (request.args.length === 0) return null;
  return JSON.stringify(request.args);
}

function startChildEngine(artifactJson: string, executionId: string, argsJson: string | null): EngineBinding {
  if (argsJson == null) {
    return new EngineBinding(artifactJson, executionId);
  }
  return new EngineBinding(artifactJson, executionId, false, argsJson);
}

function deliverChild(
  store: Store,
  state: EngineState,
  options: RunOptions & { ownerToken: string; executionId: string },
  invokeId: string | undefined,
  deferCheckpoint = false,
  branch?: string,
): "continue" | { activateChild: true } | RunResult {
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
      ...(branch ? { branch } : {}),
    });
    return "continue";
  }
  if (store.isGrouping() || deferCheckpoint) {
    return { activateChild: true };
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
          : startChildEngine(artifactJson, child.child_execution_id, child.args_json);
      })()
    : startChildEngine(artifactJson, child.child_execution_id, child.args_json);
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
    ...(branch ? { branch } : {}),
  });
  return "continue";
}

function deliverEvent(
  store: Store,
  engine: EngineBinding,
  options: RunOptions & { executionId: string },
  waitId?: string,
): "continue" | RunResult {
  const wait = waitId ? store.getWait(waitId) : store.pendingWait(options.executionId);
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
    ...(waitId ? { branch: waitId } : {}),
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

function requireEffectInput(request: Record<string, unknown>): unknown {
  if (!Object.hasOwn(request, "input")) {
    throw new Error("missing run_effect.input");
  }
  return request.input;
}

/** Canonical tagged JSON. Key order is not part of equality. */
export function canonicalEffectInput(value: unknown): string {
  if (value == null || typeof value !== "object") {
    throw new Error("unsupported effect input");
  }
  return JSON.stringify(sortJson(value));
}

export function effectInputJson(stored: string | null): string {
  if (stored == null) {
    throw new Error("effect input_json is required");
  }
  return canonicalEffectInput(JSON.parse(stored));
}

export function plainEffectInput(value: unknown): unknown {
  if (value == null || typeof value !== "object") {
    throw new Error("unsupported effect input");
  }
  return plainTagged(value);
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortJson);
  }
  if (value && typeof value === "object") {
    const sorted: Record<string, unknown> = {};
    for (const key of Object.keys(value).sort()) {
      sorted[key] = sortJson((value as Record<string, unknown>)[key]);
    }
    return sorted;
  }
  return value;
}

function plainTagged(value: unknown): unknown {
  if (!value || typeof value !== "object") {
    return {};
  }
  const tagged = value as { t?: string; v?: unknown };
  switch (tagged.t) {
    case "undefined":
      return undefined;
    case "null":
      return null;
    case "bool":
    case "number":
    case "string":
      return tagged.v;
    case "array":
      return Array.isArray(tagged.v) ? tagged.v.map(plainTagged) : [];
    case "object": {
      const plain: Record<string, unknown> = {};
      if (tagged.v && typeof tagged.v === "object") {
        for (const [key, item] of Object.entries(tagged.v as Record<string, unknown>)) {
          plain[key] = plainTagged(item);
        }
      }
      return plain;
    }
    default:
      return {};
  }
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
