import { readFileSync, writeFileSync } from "node:fs";

import { resumeExecution, startExecution, type FakeEffects } from "./host.ts";

const mode = process.env.TCC_MODE ?? "start";
const dbPath = required("TCC_DB_PATH");
const wasmPath = required("TCC_WASM_PATH");
const executionId = process.env.TCC_EXECUTION_ID ?? "first";
const ownerToken = process.env.TCC_OWNER_TOKEN ?? "owner-1";
const resultPath = process.env.TCC_RESULT_PATH;
const effectLogPath = process.env.TCC_EFFECT_LOG_PATH;
const config = process.env.TCC_CONFIG_PATH
  ? (JSON.parse(readFileSync(process.env.TCC_CONFIG_PATH, "utf8")) as {
      effects?: FakeEffects;
      failCounts?: Record<string, number>;
      eventPayload?: unknown;
      autoDeliverEvent?: boolean;
      childArtifacts?: Record<string, string>;
      cancel?: boolean;
    })
  : {};
const eventPayload = process.env.TCC_EVENT_PAYLOAD
  ? JSON.parse(process.env.TCC_EVENT_PAYLOAD)
  : config.eventPayload;
const autoDeliverEvent =
  process.env.TCC_AUTO_EVENT !== undefined ? process.env.TCC_AUTO_EVENT !== "0" : (config.autoDeliverEvent ?? true);
const effects = process.env.TCC_EFFECTS
  ? (JSON.parse(process.env.TCC_EFFECTS) as FakeEffects)
  : (config.effects ?? { generate: 42 });

const options = {
  dbPath,
  wasmPath,
  executionId,
  ownerToken,
  eventPayload,
  autoDeliverEvent,
  effectLogPath,
  effects,
  failCounts: config.failCounts,
  childArtifacts: config.childArtifacts,
  cancel: config.cancel ?? process.env.TCC_CANCEL === "1",
};

const result =
  mode === "resume"
    ? await resumeExecution(options)
    : await startExecution({
        ...options,
        artifactJson: required("TCC_ARTIFACT_JSON"),
      });

const text = JSON.stringify(result);
if (resultPath) {
  writeFileSync(resultPath, text);
}
process.stdout.write(`${text}\n`);

function required(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`missing ${name}`);
  }
  return value;
}
