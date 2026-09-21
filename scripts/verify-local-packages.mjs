#!/usr/bin/env node
import assert from "node:assert/strict";
import { mkdtempSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const dist = path.join(root, "dist-packages");
const tarballs = readdirSync(dist).filter((name) => name.endsWith(".tgz"));
const frontend = tarballs.find((name) => name.includes("frontend-typescript"));
const binding = tarballs.find((name) => name.includes("bindings-javascript"));
const host = tarballs.find((name) => name.includes("host-node"));
const primitives = tarballs.find((name) => name.includes("primitives"));
if (!frontend || !binding || !host || !primitives) {
  throw new Error(
    `missing npm packs in ${dist}: found ${tarballs.join(", ") || "(none)"}; need frontend-typescript, bindings-javascript, host-node, and primitives`,
  );
}

const dir = mkdtempSync(path.join(tmpdir(), "tcc-pack-verify-"));
const npm = spawnSync("npm", ["install", path.join(dist, primitives), path.join(dist, frontend), path.join(dist, binding), path.join(dist, host)], {
  cwd: dir,
  encoding: "utf8",
});
if (npm.status !== 0) {
  throw new Error(npm.stderr || npm.stdout);
}

const { compile, PACKAGE_VERSION, LANGUAGE_SEMANTICS_VERSION } = await import(
  path.join(dir, "node_modules/@tcc-engine/frontend-typescript/dist/index.js")
);
const { loadEngine, EngineBinding } = await import(
  path.join(dir, "node_modules/@tcc-engine/bindings-javascript/dist/index.js")
);
const { startExecution } = await import(path.join(dir, "node_modules/@tcc-engine/host-node/dist/index.js"));

assert.notEqual(PACKAGE_VERSION, LANGUAGE_SEMANTICS_VERSION);

const source = `
import { effect, waitForEvent } from "@tcc-engine/primitives";

export default async function run() {
  const result = await effect("generate", async () => {
    return generateSomething();
  });
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`;

const artifact = compile(source, { filename: "first.ts" });
await loadEngine();
const result = await startExecution({
  dbPath: path.join(dir, "tcc.db"),
  artifactJson: JSON.stringify(artifact),
  runEffect: (key) => {
    if (key !== "generate") {
      throw new Error(key);
    }
    return 42;
  },
});
assert.equal(result.status, "completed");
assert.equal(result.result?.v?.result?.v, 42);
assert.ok(EngineBinding);

console.log(`verified npm packs in ${dir}`);
