import assert from "node:assert/strict";
import test from "node:test";

import { compile, CompileError } from "./compile.ts";

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

test("rejects branching", () => {
  const source = `
import { effect } from "@trigora/sdk";
export default async function run() {
  if (true) {
    await effect("generate", async () => 1);
  }
}
`;
  assert.throws(() => compile(source), CompileError);
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

test("rejects loops, try, extra params, and missing default export", () => {
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
export default async function run() {
  try {
    await effect("generate", async () => 1);
  } catch {
    return null;
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
