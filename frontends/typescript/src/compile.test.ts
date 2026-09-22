import assert from "node:assert/strict";
import test from "node:test";

import { compile, CompileError } from "./compile.ts";
import {
  ENGINE_FORMAT_VERSION,
  FRONTEND_IDENTITY,
  LANGUAGE_SEMANTICS_VERSION,
  PACKAGE_VERSION,
} from "./types.ts";

const FIRST = `
import { effect, waitForEvent } from "@trigora/sdk";

export default async function run() {
  const result = await effect("generate", async () => {
    return generateSomething();
  });
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`;

test("compiles the first example", () => {
  const artifact = compile(FIRST, { filename: "first.ts" });
  assert.equal(artifact.program.functions[0]?.name, "run");
  assert.deepEqual(
    artifact.program.functions[0]?.instructions.map((instruction) => instruction.op),
    [
      "LoadConst",
      "Effect",
      "StoreLocal",
      "LoadConst",
      "WaitForEvent",
      "StoreLocal",
      "NewObject",
      "LoadLocal",
      "SetProp",
      "LoadLocal",
      "SetProp",
      "Return",
    ],
  );
  assert.equal(artifact.envelope.artifact_hash.length, 64);
  assert.ok(artifact.envelope.required_engine_features.includes("durable.effect"));
  assert.equal(artifact.envelope.frontend_id, FRONTEND_IDENTITY);
  assert.equal(artifact.envelope.language_semantics_version, LANGUAGE_SEMANTICS_VERSION);
  assert.equal(artifact.envelope.engine_format_version, ENGINE_FORMAT_VERSION);
  assert.equal(artifact.envelope.frontend_version, PACKAGE_VERSION);
  assert.notEqual(PACKAGE_VERSION, LANGUAGE_SEMANTICS_VERSION);
});

test("repeat compile yields the same artifact hash", () => {
  const first = compile(FIRST, { filename: "first.ts" });
  const second = compile(FIRST, { filename: "first.ts" });
  assert.equal(first.envelope.artifact_hash, second.envelope.artifact_hash);
});

test("unsupported for-of includes span, why, alternative, and version fields", () => {
  const source = `
import { effect } from "@trigora/sdk";
export default async function run() {
  const xs = await effect("xs", async () => 1);
  for (const x of xs) {
    return x;
  }
  return 0;
}
`;
  try {
    compile(source, { filename: "loop.ts" });
    assert.fail("expected CompileError");
  } catch (error) {
    assert.ok(error instanceof CompileError);
    assert.equal(error.file, "loop.ts");
    assert.equal(error.span?.file, "loop.ts");
    assert.ok((error.span?.start_line ?? 0) >= 1);
    assert.equal(error.why, "it cannot cross a durable checkpoint in the current TypeScript subset");
    assert.equal(error.alternative, "use `while`");
    assert.equal(error.frontendId, FRONTEND_IDENTITY);
    assert.equal(error.frontendVersion, PACKAGE_VERSION);
    assert.equal(error.languageSemanticsVersion, LANGUAGE_SEMANTICS_VERSION);
  }
});

test("compile errors include file and span", () => {
  try {
    compile("const x = (", { filename: "bad.ts" });
    assert.fail("expected CompileError");
  } catch (error) {
    assert.ok(error instanceof CompileError);
    assert.equal(error.file, "bad.ts");
    assert.equal(error.span?.file, "bad.ts");
    assert.equal(error.span?.start_line, 1);
    assert.match(error.message, /bad\.ts:/);
  }
});

test("accepts aliased SDK imports", () => {
  const source = `
import { effect as durableEffect, waitForEvent as wait } from "@trigora/sdk";

export default async function run() {
  const result = await durableEffect("generate", async () => 1);
  const approval = await wait("approved");
  return { result, approval };
}
`;
  const artifact = compile(source);
  assert.equal(artifact.program.functions[0]?.instructions[1]?.op, "Effect");
});

test("rejects a local function named effect", () => {
  const source = `
export default async function run() {
  async function effect(key: string, fn: () => number) {
    return fn();
  }
  const result = await effect("generate", async () => 1);
  return result;
}
`;
  assert.throws(() => compile(source), CompileError);
});

test("rejects ctx.effect", () => {
  const source = `
export default async function run(ctx: { effect: Function }) {
  await ctx.effect("generate", async () => 1);
}
`;
  assert.throws(() => compile(source), CompileError);
});

test("compiles if/else over an effect result", () => {
  const source = `
import { effect } from "@trigora/sdk";
export default async function run() {
  const flag = await effect("flag", async () => 1);
  if (flag) {
    const result = await effect("taken", async () => 42);
    return result;
  } else {
    const result = await effect("skipped", async () => 99);
    return result;
  }
}
`;
  const artifact = compile(source);
  const ops = artifact.program.functions[0]?.instructions.map((instruction) => instruction.op);
  assert.ok(ops?.includes("JumpIfFalse"));
  assert.ok(ops?.includes("Jump"));
});

test("compiles while, assignment, and unlabeled break", () => {
  const source = `
import { effect } from "@trigora/sdk";
export default async function run() {
  let go = await effect("go", async () => 1);
  while (go) {
    const x = await effect("x", async () => 42);
    go = 0;
    return x;
  }
  return 0;
}
`;
  const artifact = compile(source);
  const ops = artifact.program.functions[0]?.instructions.map((instruction) => instruction.op) ?? [];
  assert.ok(ops.includes("JumpIfFalse"));
  assert.ok(ops.includes("Jump"));
});

test("compiles nested scopes, try, sleep, and invoke", () => {
  const nested = compile(`
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const outer = await effect("outer", async () => 1);
  {
    const inner = await waitForEvent("go");
    return inner;
  }
}
`);
  assert.equal(nested.program.functions[0]?.local_count, 2);

  const caught = compile(`
import { effect } from "@trigora/sdk";
export default async function run() {
  try {
    throw "boom";
  } catch (e) {
    const ok = await effect("ok", async () => 1);
    return ok;
  }
}
`);
  assert.ok(caught.envelope.required_engine_features.includes("ts.exceptions"));

  const slept = compile(`
import { sleep, effect } from "@trigora/sdk";
export default async function run() {
  await sleep(0);
  const x = await effect("after", async () => 1);
  return x;
}
`);
  assert.ok(slept.envelope.required_engine_features.includes("durable.sleep"));

  const child = compile(`
import { invoke } from "@trigora/sdk";
export default async function run() {
  const result = await invoke("child");
  return result;
}
`);
  assert.ok(child.envelope.required_engine_features.includes("durable.invoke"));

  const cleaned = compile(`
import { effect } from "@trigora/sdk";
export default async function run() {
  let x = 0;
  try {
    const a = await effect("a", async () => 1);
    x = a;
  } finally {
    x = 1;
  }
  return x;
}
`);
  assert.ok(cleaned.envelope.required_engine_features.includes("ts.exceptions"));
});

test("rejects duplicate bindings, labeled break, for-in, and ctx", () => {
  assert.throws(
    () =>
      compile(`
import { effect } from "@trigora/sdk";
export default async function run() {
  const x = await effect("a", async () => 1);
  const x = await effect("b", async () => 2);
  return x;
}
`),
    CompileError,
  );
  assert.throws(
    () =>
      compile(`
import { effect } from "@trigora/sdk";
export default async function run() {
  loop: while (true) {
    break loop;
  }
  return 0;
}
`),
    CompileError,
  );
  assert.throws(
    () =>
      compile(`
import { effect } from "@trigora/sdk";
export default async function run() {
  const obj = await effect("o", async () => 1);
  for (const key in obj) {
    return key;
  }
  return 0;
}
`),
    CompileError,
  );
});

test("rejects workflow wrappers and unknown calls", () => {
  assert.throws(
    () =>
      compile(`
export default async function run() {
  await workflow();
}
`),
    CompileError,
  );
  assert.throws(
    () =>
      compile(`
import { effect } from "@trigora/sdk";
export default async function run() {
  const result = await effect("generate", async () => 1);
  console.log(result);
  return result;
}
`),
    CompileError,
  );
});

test("rejects non-literal effect keys", () => {
  const source = `
import { effect } from "@trigora/sdk";
export default async function run() {
  const result = await effect(["generate"][0], async () => 1);
  return result;
}
`;
  assert.throws(() => compile(source), CompileError);
});

test("rejects increment loops, extra params, and missing default export", () => {
  assert.throws(
    () =>
      compile(`
import { effect } from "@trigora/sdk";
export default async function run() {
  for (let i = 0; i < 1; i++) {
    await effect("generate", async () => 1);
  }
}
`),
    CompileError,
  );
  assert.throws(
    () =>
      compile(`
import { effect } from "@trigora/sdk";
export default async function run(ctx: unknown) {
  const result = await effect("generate", async () => 1);
  return result;
}
`),
    CompileError,
  );
  assert.throws(
    () =>
      compile(`
import { effect } from "@trigora/sdk";
export async function run() {
  const result = await effect("generate", async () => 1);
  return result;
}
`),
    CompileError,
  );
});

const FIRST_PRIMITIVES = `
import { effect, waitForEvent } from "@tcc-engine/primitives";

export default async function run() {
  const result = await effect("generate", async () => {
    return generateSomething();
  });
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`;

test("compiles the first example from primitives", () => {
  const artifact = compile(FIRST_PRIMITIVES, { filename: "first.ts" });
  assert.equal(artifact.program.functions[0]?.instructions[1]?.op, "Effect");
  assert.equal(artifact.program.functions[0]?.instructions[4]?.op, "WaitForEvent");
});

test("primitives and SDK imports produce the same artifact hash", () => {
  const fromSdk = compile(FIRST, { filename: "first.ts" });
  const fromPrimitives = compile(FIRST_PRIMITIVES, { filename: "first.ts" });
  assert.equal(fromSdk.envelope.artifact_hash, fromPrimitives.envelope.artifact_hash);
});

test("accepts aliased primitives imports", () => {
  const source = `
import { effect as durableEffect, waitForEvent as wait } from "@tcc-engine/primitives";

export default async function run() {
  const result = await durableEffect("generate", async () => 1);
  const approval = await wait("approved");
  return { result, approval };
}
`;
  const artifact = compile(source);
  assert.equal(artifact.program.functions[0]?.instructions[1]?.op, "Effect");
});

test("rejects imports from other modules", () => {
  assert.throws(
    () =>
      compile(`
import { effect } from "somewhere-else";
export default async function run() {
  const result = await effect("generate", async () => 1);
  return result;
}
`),
    CompileError,
  );
});

test("accepts mixed primitives and SDK imports in one file", () => {
  const source = `
import { effect } from "@tcc-engine/primitives";
import { waitForEvent } from "@trigora/sdk";

export default async function run() {
  const result = await effect("generate", async () => 1);
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`;
  const artifact = compile(source, { filename: "first.ts" });
  assert.equal(artifact.program.functions[0]?.instructions[1]?.op, "Effect");
  assert.equal(artifact.program.functions[0]?.instructions[4]?.op, "WaitForEvent");
});

test("lowers direct await Promise.all and rejects the other shapes", () => {
  const source = `
import { effect } from "@tcc-engine/primitives";
export default async function run() {
  const [a, b] = await Promise.all([
    effect("a", async () => 1),
    effect("b", async () => 2),
  ]);
  return a;
}
`;
  const artifact = compile(source, { filename: "all.ts" });
  const ops = artifact.program.functions[0]?.instructions.map((instruction) => instruction.op);
  assert.ok(ops?.includes("Fork"));
  assert.ok(ops?.includes("JoinAll"));
  assert.ok(artifact.envelope.required_engine_features.includes("durable.concurrent_group"));
  assert.throws(
    () =>
      compile(
        `
import { effect } from "@tcc-engine/primitives";
export default async function run() {
  const p = Promise.all([effect("a", async () => 1)]);
  return p;
}
`,
        { filename: "stored.ts" },
      ),
    /await Promise.all directly/,
  );
  assert.throws(
    () =>
      compile(
        `
export default async function run() {
  return await Promise.race([1]);
}
`,
        { filename: "race.ts" },
      ),
    /Promise.race branches must be durable calls/,
  );
  const race = compile(
    `
import { effect, sleep } from "@tcc-engine/primitives";
export default async function run() {
  return await Promise.race([
    effect("charge", async () => 1),
    sleep(5000),
  ]);
}
`,
    { filename: "race.ts" },
  );
  const raceOps = race.program.functions[0]?.instructions.map((instruction) => instruction.op);
  assert.ok(raceOps?.includes("Fork"));
  assert.ok(raceOps?.includes("JoinAny"));
  assert.ok(!raceOps?.includes("JoinAll"));
  assert.throws(
    () =>
      compile(
        `
import { effect } from "@tcc-engine/primitives";
export default async function run() {
  const p = Promise.race([effect("a", async () => 1)]);
  return p;
}
`,
        { filename: "stored-race.ts" },
      ),
    /await Promise.race directly/,
  );
  assert.throws(
    () =>
      compile(
        `
export default async function run() {
  return await Promise.any([1]);
}
`,
        { filename: "any.ts" },
      ),
    /Promise.any is not supported/,
  );
});
