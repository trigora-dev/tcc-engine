import { EngineBinding, loadEngine, type Outcome } from "../../../bindings/javascript/src/index.ts";

export type FakeEffects = Record<string, unknown>;

export type RunOptions = {
  artifactJson: string;
  wasmPath: string;
  executionId?: string;
  effects?: FakeEffects;
  eventPayload?: unknown;
  budget?: number;
};

export type RunResult = {
  status: string;
  result: unknown;
  continuationJson: string;
  hostRequests: Record<string, unknown>[];
};

export async function runProgram(options: RunOptions): Promise<RunResult> {
  await loadEngine(options.wasmPath);
  const engine = new EngineBinding(options.artifactJson, options.executionId ?? "first");
  const effects = options.effects ?? { generate: 42 };
  const eventPayload = options.eventPayload ?? "ok";
  const budget = options.budget ?? 256;
  const hostRequests: Record<string, unknown>[] = [];
  let revision = 0;

  for (;;) {
    const outcome = engine.runUntilHost(budget);
    const next = handleOutcome(engine, outcome, effects, eventPayload, hostRequests, () => {
      revision += 1;
      return revision;
    });
    if (next === "continue") {
      continue;
    }
    return {
      status: next.status,
      result: next.result,
      continuationJson: engine.continuationJson(),
      hostRequests,
    };
  }
}

function handleOutcome(
  engine: EngineBinding,
  outcome: Outcome,
  effects: FakeEffects,
  eventPayload: unknown,
  hostRequests: Record<string, unknown>[],
  nextRevision: () => number,
): "continue" | { status: string; result: unknown } {
  switch (outcome.type) {
    case "completed":
      return { status: "completed", result: outcome.result };
    case "failed":
      throw new Error(outcome.message);
    case "budget_exhausted":
      throw new Error("instruction budget exhausted");
    case "suspended":
      engine.applyHostResponse({
        type: "event_payload",
        value: encodeValue(eventPayload),
      });
      return "continue";
    case "host": {
      const request = outcome.request;
      hostRequests.push(request);
      switch (request.type) {
        case "run_effect": {
          const key = String(request.key);
          if (!(key in effects)) {
            throw new Error(`no fake effect for \`${key}\``);
          }
          engine.applyHostResponse({
            type: "effect_result",
            value: encodeValue(effects[key]),
          });
          return "continue";
        }
        case "persist_effect":
        case "register_wait":
        case "register_timer":
        case "create_child":
          engine.applyHostResponse({ type: "ack" });
          return "continue";
        case "persist_checkpoint":
          engine.applyHostResponse({
            type: "persist_confirmed",
            revision: nextRevision(),
          });
          return "continue";
        default:
          throw new Error(`unsupported host request \`${String(request.type)}\``);
      }
    }
    default:
      throw new Error("unknown engine outcome");
  }
}

function encodeValue(value: unknown): { t: string; v?: unknown } {
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
