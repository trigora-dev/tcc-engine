import { writeFileSync } from "node:fs";

import { resumeExecution, startExecution } from "./host.ts";

const mode = process.env.TCC_MODE ?? "start";
const dbPath = required("TCC_DB_PATH");
const wasmPath = required("TCC_WASM_PATH");
const executionId = process.env.TCC_EXECUTION_ID ?? "first";
const ownerToken = process.env.TCC_OWNER_TOKEN ?? "owner-1";
const resultPath = process.env.TCC_RESULT_PATH;
const effectLogPath = process.env.TCC_EFFECT_LOG_PATH;
const eventPayload = process.env.TCC_EVENT_PAYLOAD
  ? JSON.parse(process.env.TCC_EVENT_PAYLOAD)
  : undefined;
const autoDeliverEvent = process.env.TCC_AUTO_EVENT !== "0";

const options = {
  dbPath,
  wasmPath,
  executionId,
  ownerToken,
  eventPayload,
  autoDeliverEvent,
  effectLogPath,
  effects: { generate: 42 },
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
