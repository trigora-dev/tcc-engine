import assert from "node:assert/strict";
import test from "node:test";

import { compile } from "../../../frontends/typescript/src/compile.ts";
import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { assertConformance, createWorkspace, runUninterrupted } from "./conformance.ts";
import { resumeExecution, startExecution } from "./host.ts";
import { wasmPath } from "./conformance.ts";
import { Store } from "./store.ts";

const ALL = `
import { effect, waitForEvent } from "@tcc-engine/primitives";
export default async function run() {
  const [a, b] = await Promise.all([
    effect("a", async () => 1),
    effect("b", async () => 2),
  ]);
  const approval = await waitForEvent("go");
  return { a, approval };
}
`;

const TIMERS = `
import { sleep } from "@tcc-engine/primitives";
export default async function run() {
  const pair = await Promise.all([sleep(5), sleep(5)]);
  return pair;
}
`;

test("Promise.all survives crashes and drops the unused aggregate element", async () => {
  await assertConformance({
    source: ALL,
    effects: { a: 1, b: 2 },
    eventPayload: "ok",
    expectedResult: {
      t: "object",
      v: {
        a: { t: "number", v: 1 },
        approval: { t: "string", v: "ok" },
      },
    },
    crashAts: ["after_persist_effect", "after_wait_checkpoint", "before_event_payload"],
  });
});

test("two sleeps with the same deadline are two timers", async () => {
  const result = await runUninterrupted({ source: TIMERS, effects: {} });
  assert.equal(result.status, "completed");
  assert.deepEqual(result.result, {
    t: "array",
    v: [{ t: "undefined" }, { t: "undefined" }],
  });
});

test("unused Promise.all element is undefined at the next wait", async () => {
  const paths = createWorkspace();
  const artifactJson = canonicalStringify(compile(ALL, { filename: "all.ts" }));
  const started = await startExecution({
    dbPath: paths.dbPath,
    wasmPath,
    artifactJson,
    effects: { a: 1, b: 2 },
    autoDeliverEvent: false,
    executionId: "live",
  });
  assert.equal(started.status, "suspended");
  const parsed = JSON.parse(started.continuationJson ?? "{}") as {
    frames: Array<{ locals: Array<{ t: string; v?: unknown }> }>;
  };
  assert.equal(parsed.frames[0]?.locals[0]?.v, 1);
  assert.equal(parsed.frames[0]?.locals[1]?.t, "undefined");
});

test("one event resolves one same-name wait", async () => {
  const source = `
import { waitForEvent } from "@tcc-engine/primitives";
export default async function run() {
  const pair = await Promise.all([
    waitForEvent("ready"),
    waitForEvent("ready"),
  ]);
  return pair;
}
`;
  const paths = createWorkspace();
  const artifactJson = canonicalStringify(compile(source, { filename: "waits.ts" }));
  const started = await startExecution({
    dbPath: paths.dbPath,
    wasmPath,
    artifactJson,
    autoDeliverEvent: false,
    executionId: "waits",
  });
  assert.equal(started.status, "suspended");
  const store = new Store(paths.dbPath);
  store.enqueueEvent("waits", "ready", JSON.stringify({ t: "string", v: "one" }));
  store.close();
  const resumed = await resumeExecution({
    dbPath: paths.dbPath,
    wasmPath,
    artifactJson,
    executionId: "waits",
    autoDeliverEvent: false,
  });
  assert.equal(resumed.status, "suspended");
  const after = new Store(paths.dbPath);
  const pending = after.pendingWait("waits");
  after.close();
  assert.ok(pending);
});
