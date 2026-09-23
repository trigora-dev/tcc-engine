/**
 * Permanent compatibility run for the ordinary corpus.
 *
 * Every program must pass on the native engine, WASM, and the Python binding.
 * Where a durable boundary exists, resume must return the same result with one frame.
 */
import { readFileSync, existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { spawnSync } from "node:child_process";

import { compile as compileTypeScript } from "../../frontends/typescript/src/compile.ts";
import { canonicalStringify } from "../../frontends/typescript/src/canonical.ts";
import { EngineBinding, loadEngine } from "../../bindings/javascript/src/index.ts";

const root = path.dirname(fileURLToPath(import.meta.url));

type Tagged = { t: string; v?: unknown };
type Program = {
  id: string;
  typescript?: string;
  python?: string;
  rust?: string;
  args?: Tagged[];
  effects?: Record<string, Tagged>;
  events?: Record<string, Tagged>;
  child?: Tagged;
  expected: Tagged;
};

type Driver = {
  run(budget?: number): Outcome;
  apply(response: Record<string, unknown>): void;
  continuation(): string;
  resume(artifact: string, continuation: string): Driver;
};

type Outcome =
  | { type: "host"; request: Record<string, unknown> }
  | { type: "completed"; result: unknown }
  | { type: "failed"; message: string }
  | { type: "suspended" }
  | { type: "cancelled" }
  | { type: "budget_exhausted" };

export function nonIoLines(source: string, language: "ts" | "py" | "rs"): number {
  const lines = source.split("\n");
  let count = 0;
  let comment = false;
  let callback = 0;
  for (const raw of lines) {
    const line = raw.trim();
    if (language === "ts" || language === "rs") {
      if (comment) {
        if (line.includes("*/")) {
          comment = false;
        }
        continue;
      }
      if (line.startsWith("/*")) {
        if (!line.includes("*/")) {
          comment = true;
        }
        continue;
      }
      if (line.startsWith("//") || line === "") {
        continue;
      }
    } else if (line.startsWith("#") || line === "") {
      continue;
    }
    if (language === "rs" && line.startsWith("use tcc_rust_prelude")) {
      continue;
    }
    if (/^import\b/.test(line) && /@tcc-engine\/primitives|@trigora\/sdk|tcc_engine\.primitives|trigora/.test(line)) {
      continue;
    }
    if (/await\s+(effect|waitForEvent|wait_for_event|sleep|invoke)\b/.test(line) || /\.(await\?)|\.await\?/.test(line)) {
      continue;
    }
    if (/async\s*\(\)\s*=>|lambda\b/.test(line)) {
      callback += (line.match(/async\s*\(\)\s*=>|lambda\b/g) ?? []).length;
    }
    if (callback > 0) {
      callback += (line.match(/\{/g) ?? []).length;
      callback -= (line.match(/\}/g) ?? []).length;
      if (callback < 0) {
        callback = 0;
      }
      continue;
    }
    count += 1;
  }
  return count;
}

export async function drive(engine: Driver, program: Program): Promise<unknown> {
  const effects = program.effects ?? {};
  const events = program.events ?? {};
  for (let step = 0; step < 10000; step += 1) {
    const outcome = engine.run(100000);
    if (outcome.type === "completed") {
      return outcome.result;
    }
    if (outcome.type === "failed") {
      throw new Error(outcome.message);
    }
    if (outcome.type === "suspended") {
      const response = wake(program);
      if (!response) {
        throw new Error("suspended without an event");
      }
      engine.apply(response);
      continue;
    }
    if (outcome.type !== "host") {
      throw new Error(`unexpected outcome ${outcome.type}`);
    }
    const request = outcome.request;
    const kind = String(request.type);
    if (kind === "run_effect") {
      const key = String(request.key);
      const value = effects[key];
      if (value === undefined) {
        throw new Error(`missing effect ${key}`);
      }
      engine.apply({ type: "effect_result", value });
    } else if (kind === "persist_checkpoint") {
      engine.apply({ type: "persist_confirmed", revision: request.revision });
    } else if (
      kind === "persist_effect" ||
      kind === "register_wait" ||
      kind === "register_timer" ||
      kind === "create_child"
    ) {
      engine.apply({ type: "ack" });
    } else {
      throw new Error(`unexpected host request ${kind}`);
    }
  }
  throw new Error("drive did not finish");
}

function wake(program: Program): Record<string, unknown> | undefined {
  if (program.child) {
    return { type: "child_result", value: program.child };
  }
  const name = Object.keys(program.events ?? {})[0];
  if (!name) {
    return undefined;
  }
  return { type: "event_payload", value: program.events?.[name] };
}

function valuesEqual(actual: unknown, expected: unknown): boolean {
  return canonicalStringify(actual) === canonicalStringify(expected);
}

function wasmEngine(artifact: string, args: Tagged[]): Driver {
  const argsJson = args.length === 0 ? undefined : canonicalStringify(args);
  const binding = new EngineBinding(artifact, "ordinary", false, argsJson);
  return {
    run: (budget) => binding.runUntilHost(budget) as Outcome,
    apply: (response) => binding.applyHostResponse(response),
    continuation: () => binding.continuationJson(),
    resume: (nextArtifact, continuation) => wasmResume(nextArtifact, continuation),
  };
}

function wasmResume(artifact: string, continuation: string): Driver {
  const binding = EngineBinding.resume(artifact, continuation);
  return {
    run: (budget) => binding.runUntilHost(budget) as Outcome,
    apply: (response) => binding.applyHostResponse(response),
    continuation: () => binding.continuationJson(),
    resume: (nextArtifact, next) => wasmResume(nextArtifact, next),
  };
}

type LayerResult = {
  lines: number;
  unchanged: number;
  programs: number;
  passed: number;
  failures: string[];
};

async function scoreLayer(programs: Program[]): Promise<LayerResult> {
  const result: LayerResult = { lines: 0, unchanged: 0, programs: 0, passed: 0, failures: [] };
  for (const program of programs) {
    const files: Array<{ language: "ts" | "py" | "rs"; file: string }> = [];
    if (program.typescript) {
      files.push({ language: "ts", file: program.typescript });
    }
    if (program.python) {
      files.push({ language: "py", file: program.python });
    }
    if (program.rust) {
      files.push({ language: "rs", file: program.rust });
    }
    for (const file of files) {
      result.programs += 1;
      const source = readFileSync(path.join(root, file.file), "utf8");
      const weight = nonIoLines(source, file.language);
      result.lines += weight;
      try {
        const json =
          file.language === "ts"
            ? canonicalStringify(compileTypeScript(source, { filename: path.basename(file.file) }))
            : file.language === "py"
              ? compilePython(source, path.basename(file.file))
              : compileRust(file.file);
        const wasm = await drive(wasmEngine(json, program.args ?? []), program);
        const python = driveNativePython(json, program);
        const native = driveNative(json, program);
        if (
          !valuesEqual(wasm, program.expected) ||
          !valuesEqual(python, program.expected) ||
          !valuesEqual(native, program.expected)
        ) {
          result.failures.push(
            `${program.id} ${file.file}: wasm ${canonicalStringify(wasm)} native ${canonicalStringify(native)} python ${canonicalStringify(python)}`,
          );
          continue;
        }
        if (program.events || program.effects || program.child) {
          await assertSingleFrameResume(json, program);
        }
        result.unchanged += weight;
        result.passed += 1;
      } catch (error) {
        result.failures.push(`${program.id} ${file.file}: ${error instanceof Error ? error.message : error}`);
      }
    }
  }
  return result;
}

function reportLayer(name: string, result: LayerResult): void {
  const lineRate = result.lines === 0 ? 1 : result.unchanged / result.lines;
  const programRate = result.programs === 0 ? 1 : result.passed / result.programs;
  console.log(
    `${name} non-I/O lines ${result.unchanged}/${result.lines} (${(lineRate * 100).toFixed(1)}%)`,
  );
  console.log(`${name} programs ${result.passed}/${result.programs} (${(programRate * 100).toFixed(1)}%)`);
}

async function main(): Promise<void> {
  const wasm = path.resolve(root, "../../target/wasm32-unknown-unknown/release/tcc_wasm.wasm");
  await loadEngine(wasm);
  const manifest = JSON.parse(readFileSync(path.join(root, "manifest.json"), "utf8")) as {
    programs: Program[];
  };
  const ids = new Set<string>();
  for (const program of manifest.programs) {
    if (ids.has(program.id)) {
      throw new Error(`duplicate program id ${program.id}`);
    }
    ids.add(program.id);
  }
  const result = await scoreLayer(manifest.programs);
  reportLayer("ordinary", result);
  for (const failure of result.failures) {
    console.log(`FAIL ${failure}`);
  }
  if (result.passed !== result.programs) {
    process.exitCode = 1;
  }
}

async function assertSingleFrameResume(artifact: string, program: Program): Promise<void> {
  const engine = wasmEngine(artifact, program.args ?? []);
  for (let step = 0; step < 10000; step += 1) {
    const outcome = engine.run(100000);
    if (outcome.type === "suspended") {
      await resumeFromHere(engine, artifact, program, true);
      return;
    }
    if (outcome.type === "completed") {
      return;
    }
    if (outcome.type !== "host") {
      throw new Error(`${program.id} resume setup ${outcome.type}`);
    }
    const kind = String(outcome.request.type);
    if (kind === "persist_checkpoint") {
      engine.apply({ type: "persist_confirmed", revision: outcome.request.revision });
    } else if (kind === "persist_effect" && !program.events) {
      engine.apply({ type: "ack" });
      await resumeFromHere(engine, artifact, program, false);
      return;
    } else if (kind === "register_wait" || kind === "persist_effect" || kind === "create_child") {
      engine.apply({ type: "ack" });
    } else if (kind === "run_effect") {
      const key = String(outcome.request.key);
      engine.apply({ type: "effect_result", value: program.effects?.[key] });
    } else {
      throw new Error(`${program.id} unexpected ${kind} before resume`);
    }
  }
}

const pythonEnginePath = path.join(root, "score_engine.py");

async function resumeFromHere(
  engine: Driver,
  artifact: string,
  program: Program,
  deliverEvent: boolean,
): Promise<void> {
  const continuation = JSON.parse(engine.continuation()) as { frames?: unknown[] };
  if ((continuation.frames?.length ?? 0) !== 1) {
    throw new Error(`${program.id} resumed continuation has a helper frame`);
  }
  const resumed = engine.resume(artifact, engine.continuation());
  if (deliverEvent) {
    const response = wake(program);
    if (!response) {
      throw new Error(`${program.id} resume without an event`);
    }
    resumed.apply(response);
  }
  const result = await drive(resumed, program);
  if (!valuesEqual(result, program.expected)) {
    throw new Error(`${program.id} resume result ${canonicalStringify(result)}`);
  }
}

function pythonEngine(message: Record<string, unknown>): unknown {
  const result = spawnSync("python3", [pythonEnginePath], {
    input: JSON.stringify(message),
    encoding: "utf8",
    env: {
      ...process.env,
      PYTHONPATH: path.resolve(root, "../../bindings/python/python"),
      PYTHONDONTWRITEBYTECODE: "1",
    },
  });
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "python engine failed");
  }
  return JSON.parse(result.stdout);
}

function compilePython(source: string, filename: string): string {
  const artifact = pythonEngine({ mode: "compile", source, filename });
  return canonicalStringify(artifact);
}

function compileRust(file: string): string {
  const bin = rustCompiler();
  const result = spawnSync(bin, [path.join(root, file)], { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || `rust compiler failed (${bin})`);
  }
  return canonicalStringify(JSON.parse(result.stdout));
}

function rustCompiler(): string {
  if (process.env.TCC_RUSTC) {
    return process.env.TCC_RUSTC;
  }
  const release = path.resolve(root, "../../target/release/tcc-rust-compile");
  const debug = path.resolve(root, "../../target/debug/tcc-rust-compile");
  if (existsSync(release)) {
    return release;
  }
  if (existsSync(debug)) {
    return debug;
  }
  return release;
}

function driveNativePython(artifact: string, program: Program): unknown {
  return pythonEngine({
    mode: "run",
    artifact,
    args: program.args ?? [],
    effects: program.effects ?? {},
    events: program.events ?? {},
    child: program.child ?? null,
  });
}

function driveNative(artifact: string, program: Program): unknown {
  const bin =
    process.env.TCC_DRIVE ??
    path.resolve(root, "../../target/release/tcc-drive");
  const result = spawnSync(bin, [], {
    input: JSON.stringify({
      artifact,
      args: program.args ?? [],
      effects: program.effects ?? {},
      events: program.events ?? {},
      child: program.child ?? null,
    }),
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || `native driver failed (${bin})`);
  }
  return JSON.parse(result.stdout);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error: unknown) => {
    console.error(error);
    process.exitCode = 1;
  });
}
