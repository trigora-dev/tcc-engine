import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import { runProgram } from "./host.ts";

const SOURCE = `
import { effect, waitForEvent } from "@trigora/sdk";

export default async function run() {
  const result = await effect("generate", async () => {
    return generateSomething();
  });
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`;

const wasmPath = path.resolve(
  fileURLToPath(
    new URL("../../../target/wasm32-unknown-unknown/release/tcc_wasm.wasm", import.meta.url),
  ),
);

test("memory host completes the first example on WASM", async () => {
  const artifact = compile(SOURCE, { filename: "first.ts" });
  const result = await runProgram({
    artifactJson: canonicalStringify(artifact),
    wasmPath,
  });
  assert.equal(result.status, "completed");
  assert.deepEqual(result.result, {
    t: "object",
    v: {
      approval: { t: "string", v: "ok" },
      result: { t: "number", v: 42 },
    },
  });
  assert.deepEqual(
    result.hostRequests.map((request) => request.type),
    [
      "run_effect",
      "persist_effect",
      "persist_checkpoint",
      "register_wait",
      "persist_checkpoint",
      "persist_checkpoint",
      "persist_checkpoint",
    ],
  );
  assert.equal(result.hostRequests[0]?.idempotency_key, "first:generate");
  assert.equal(result.hostRequests[3]?.wait_id, "first:approved::4");
  const continuation = JSON.parse(result.continuationJson) as {
    status: string;
    result: unknown;
  };
  assert.equal(continuation.status, "completed");
  assert.deepEqual(continuation.result, result.result);
});
