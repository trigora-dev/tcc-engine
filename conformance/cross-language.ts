/**
 * Cross-language invoke. Each parent and child are separate artifacts.
 * TypeScript calls Rust, Rust calls Python, and Python calls Rust.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { EngineBinding, loadEngine } from "../bindings/javascript/src/index.ts";
import { canonicalStringify } from "../frontends/typescript/src/canonical.ts";
import { compile as compileTypeScript } from "../frontends/typescript/src/compile.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const cases = path.join(here, "cross-language");

type Outcome =
  | { type: "completed"; result: unknown }
  | { type: "failed"; message: string }
  | { type: "suspended" }
  | { type: "host"; request: Record<string, unknown> };

const ok = (value: number) => ({
  t: "object",
  v: {
    $tag: { t: "string", v: "Ok" },
    $0: { t: "number", v: value },
  },
});

function rustCompiler(): string {
  if (process.env.TCC_RUSTC) {
    return process.env.TCC_RUSTC;
  }
  const release = path.join(root, "target/release/tcc-rust-compile");
  if (existsSync(release)) {
    return release;
  }
  return path.join(root, "target/debug/tcc-rust-compile");
}

function compileRust(file: string): string {
  const bin = rustCompiler();
  const result = spawnSync(bin, [file], { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || `rust compiler failed (${bin})`);
  }
  return canonicalStringify(JSON.parse(result.stdout));
}

function compilePython(file: string): string {
  const source = readFileSync(file, "utf8");
  const result = spawnSync("python3", [path.join(here, "cross_language_engine.py")], {
    input: JSON.stringify({ mode: "compile", source, filename: path.basename(file) }),
    encoding: "utf8",
    env: {
      ...process.env,
      PYTHONPATH: path.join(root, "bindings/python/python"),
      PYTHONDONTWRITEBYTECODE: "1",
    },
  });
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "python compiler failed");
  }
  return result.stdout;
}

function compileTypeScriptFile(file: string): string {
  const source = readFileSync(file, "utf8");
  return canonicalStringify(compileTypeScript(source, { filename: path.basename(file) }));
}

function wasmEngine(artifact: string, args?: unknown[], isolated = false) {
  const argsJson = args && args.length > 0 ? canonicalStringify(args) : undefined;
  const binding = new EngineBinding(artifact, "cross-language", isolated, argsJson);
  return {
    run: () => binding.runUntilHost(100000) as Outcome,
    apply: (response: Record<string, unknown>) => binding.applyHostResponse(response),
  };
}

function driveWasm(parent: string, child: string): unknown {
  return drive(wasmEngine(parent), child, (artifact, args) => wasmEngine(artifact, args, true));
}

function drive(
  engine: { run: () => Outcome; apply: (response: Record<string, unknown>) => void },
  child: string | undefined,
  start: (artifact: string, args: unknown[]) => { run: () => Outcome; apply: (response: Record<string, unknown>) => void },
): unknown {
  let childResult: unknown;
  for (let step = 0; step < 10000; step += 1) {
    const outcome = engine.run();
    if (outcome.type === "completed") {
      return outcome.result;
    }
    if (outcome.type === "failed") {
      throw new Error(outcome.message);
    }
    if (outcome.type === "suspended") {
      if (childResult === undefined) {
        throw new Error("suspended without a child result");
      }
      engine.apply({ type: "child_result", value: childResult });
      continue;
    }
    const kind = String(outcome.request.type);
    if (kind === "persist_checkpoint") {
      engine.apply({ type: "persist_confirmed", revision: outcome.request.revision });
    } else if (kind === "create_child") {
      if (!child) {
        throw new Error("child artifact missing");
      }
      engine.apply({ type: "ack" });
      const args = Array.isArray(outcome.request.args) ? outcome.request.args : [];
      childResult = drive(start(child, args), undefined, start);
    } else if (kind === "persist_effect" || kind === "register_wait" || kind === "register_timer") {
      engine.apply({ type: "ack" });
    } else {
      throw new Error(`unexpected host request ${kind}`);
    }
  }
  throw new Error("drive did not finish");
}

function drivePython(parent: string, child: string): unknown {
  const result = spawnSync("python3", [path.join(here, "cross_language_engine.py")], {
    input: JSON.stringify({ mode: "invoke", parent, child }),
    encoding: "utf8",
    env: {
      ...process.env,
      PYTHONPATH: path.join(root, "bindings/python/python"),
      PYTHONDONTWRITEBYTECODE: "1",
    },
  });
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "python engine failed");
  }
  return JSON.parse(result.stdout);
}

const pairs = [
  {
    name: "ts → rust",
    parent: compileTypeScriptFile(path.join(cases, "ts-calls-rust.ts")),
    child: compileRust(path.join(cases, "rust-child.rs")),
    expected: { t: "number", v: 7 },
  },
  {
    name: "rust → python",
    parent: compileRust(path.join(cases, "rust-calls-python.rs")),
    child: compilePython(path.join(cases, "python-child.py")),
    expected: ok(7),
  },
  {
    name: "python → rust",
    parent: compilePython(path.join(cases, "python-calls-rust.py")),
    child: compileRust(path.join(cases, "rust-child.rs")),
    expected: { t: "number", v: 7 },
  },
];

const wasm = path.join(root, "target/wasm32-unknown-unknown/release/tcc_wasm.wasm");
await loadEngine(wasm);
for (const pair of pairs) {
  const wasmResult = driveWasm(pair.parent, pair.child);
  const pythonResult = drivePython(pair.parent, pair.child);
  assert.equal(canonicalStringify(wasmResult), canonicalStringify(pair.expected), `${pair.name} wasm`);
  assert.equal(canonicalStringify(pythonResult), canonicalStringify(pair.expected), `${pair.name} python`);
  console.log(`${pair.name} ok`);
}
