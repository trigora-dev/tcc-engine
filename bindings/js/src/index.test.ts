import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { engineFormatVersion, loadEngine, EngineBinding } from "./index.ts";

const wasmPath = path.resolve(
  fileURLToPath(new URL("../../../target/wasm32-unknown-unknown/release/tcc_wasm.wasm", import.meta.url)),
);

test("loads the wasm module", async () => {
  await loadEngine(wasmPath);
  assert.equal(engineFormatVersion(), 1);
});

test("resume rejects invalid continuation json", async () => {
  await loadEngine(wasmPath);
  assert.throws(() => EngineBinding.resume("{}", "{}"));
});
