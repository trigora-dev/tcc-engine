import test from "node:test";

import { compile } from "../../../frontends/typescript/src/compile.ts";
import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { assertConformance, killAndResume, runUninterrupted } from "./conformance.ts";

const IF_ELSE = `
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

const LOOP = `
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

const SCOPES = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const outer = await effect("outer", async () => 1);
  {
    const inner = await waitForEvent("go");
    return { outer, inner };
  }
}
`;

const MULTI = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const a = await effect("a", async () => 1);
  const b = await effect("b", async () => 2);
  const ev = await waitForEvent("go");
  return { a, b, ev };
}
`;

const VALUES = `
import { effect } from "@trigora/sdk";
export default async function run() {
  const n = await effect("n", async () => 3);
  if (n === 3) {
    const yes = !false;
    return { ok: yes, xs: [n, 1] };
  }
  return { ok: false, xs: [] };
}
`;

const TRY = `
import { effect } from "@trigora/sdk";
export default async function run() {
  try {
    throw "boom";
  } catch (e) {
    const ok = await effect("ok", async () => 1);
    return ok;
  }
}
`;

const RETRY = `
import { effect } from "@trigora/sdk";
export default async function run() {
  const x = await effect("flaky", async () => 7);
  return x;
}
`;

const SLEEP = `
import { sleep, effect } from "@trigora/sdk";
export default async function run() {
  await sleep(0);
  const x = await effect("after", async () => 1);
  return x;
}
`;

const CANCEL = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const a = await effect("a", async () => 1);
  const b = await waitForEvent("never");
  return b;
}
`;

const INVOKE = `
import { invoke } from "@trigora/sdk";
export default async function run() {
  const result = await invoke("child");
  return result;
}
`;

const CHILD = `
import { effect } from "@trigora/sdk";
export default async function run() {
  const result = await effect("child_work", async () => 7);
  return result;
}
`;

test("if/else recovery skips the untaken branch", async () => {
  await assertConformance({
    source: IF_ELSE,
    effects: { flag: 1, taken: 42, skipped: 99 },
    expectedResult: { t: "number", v: 42 },
    expectedEffectLog: ["flag", "taken"],
    crashAts: ["after_persist_effect:flag", "after_persist_effect:taken", "after_persist_checkpoint:1"],
  });
});

test("loops journal-skip the same effect key", async () => {
  await assertConformance({
    source: LOOP,
    effects: { go: 1, x: 42 },
    expectedResult: { t: "number", v: 42 },
    expectedEffectLog: ["go", "x"],
    crashAts: ["after_persist_effect:go", "after_persist_effect:x"],
  });
});

test("nested scope locals survive wait plus SIGKILL", async () => {
  await assertConformance({
    source: SCOPES,
    effects: { outer: 1 },
    eventPayload: "ok",
    expectedResult: {
      t: "object",
      v: {
        inner: { t: "string", v: "ok" },
        outer: { t: "number", v: 1 },
      },
    },
    expectedEffectLog: ["outer"],
    crashAts: ["after_persist_effect:outer", "after_wait_checkpoint"],
  });
});

test("multiple durable operations keep frozen identities", async () => {
  await assertConformance({
    source: MULTI,
    effects: { a: 1, b: 2 },
    eventPayload: "ok",
    expectedResult: {
      t: "object",
      v: {
        a: { t: "number", v: 1 },
        b: { t: "number", v: 2 },
        ev: { t: "string", v: "ok" },
      },
    },
    expectedEffectLog: ["a", "b"],
    crashAts: ["after_persist_effect:a", "after_persist_effect:b", "after_wait_checkpoint"],
  });
});

test("richer values compare and build nested structures", async () => {
  await assertConformance({
    source: VALUES,
    effects: { n: 3 },
    expectedResult: {
      t: "object",
      v: {
        ok: { t: "bool", v: true },
        xs: {
          t: "array",
          v: [
            { t: "number", v: 3 },
            { t: "number", v: 1 },
          ],
        },
      },
    },
    expectedEffectLog: ["n"],
    crashAts: ["after_persist_effect:n"],
  });
});

test("try/catch recovers after the handler effect", async () => {
  await assertConformance({
    source: TRY,
    effects: { ok: 1 },
    expectedResult: { t: "number", v: 1 },
    expectedEffectLog: ["ok"],
    crashAts: ["after_persist_effect:ok"],
  });
});

test("try/finally runs cleanup after a durable op", async () => {
  await assertConformance({
    source: `
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
`,
    effects: { a: 1 },
    expectedResult: { t: "number", v: 1 },
    expectedEffectLog: ["a"],
    crashAts: ["after_persist_effect:a"],
  });
});

test("failed effect retries the same idempotency key", async () => {
  const clean = await runUninterrupted({
    source: RETRY,
    effects: { flaky: 7 },
    failCounts: { flaky: 1 },
  });
  if (clean.status !== "completed" || JSON.stringify(clean.result) !== JSON.stringify({ t: "number", v: 7 })) {
    throw new Error(`retry uninterrupted failed: ${JSON.stringify(clean)}`);
  }
  if (JSON.stringify(clean.effectLog) !== JSON.stringify(["flaky", "flaky"])) {
    throw new Error(`retry log ${JSON.stringify(clean.effectLog)}`);
  }
  const recovered = await killAndResume({
    source: RETRY,
    crashAt: "after_persist_effect:flaky",
    effects: { flaky: 7 },
    failCounts: { flaky: 1 },
  });
  if (recovered.resumed.status !== "completed") {
    throw new Error(`retry resume ${recovered.resumed.status}`);
  }
  if (JSON.stringify(recovered.effectLog) !== JSON.stringify(["flaky", "flaky"])) {
    throw new Error(`retry recovered log ${JSON.stringify(recovered.effectLog)}`);
  }
});

test("sleep survives SIGKILL then timer fire", async () => {
  await assertConformance({
    source: SLEEP,
    effects: { after: 1 },
    expectedResult: { t: "number", v: 1 },
    expectedEffectLog: ["after"],
    crashAts: ["after_register_timer", "after_wait_checkpoint", "after_persist_effect:after"],
  });
});

test("cancel takes effect at the next durable boundary", async () => {
  const clean = await runUninterrupted({
    source: CANCEL,
    effects: { a: 1 },
    cancel: true,
    autoDeliverEvent: false,
  });
  if (clean.status !== "cancelled") {
    throw new Error(`expected cancelled, got ${clean.status}`);
  }
  const recovered = await killAndResume({
    source: CANCEL,
    crashAt: "after_wait_checkpoint",
    effects: { a: 1 },
    cancel: true,
    autoDeliverEvent: false,
  });
  if (recovered.resumed.status !== "cancelled") {
    throw new Error(`resume expected cancelled, got ${recovered.resumed.status}`);
  }
  if (JSON.stringify(recovered.effectLog) !== JSON.stringify(["a"])) {
    throw new Error(`cancel log ${JSON.stringify(recovered.effectLog)}`);
  }
});

test("child invoke recovers parent wait and child result", async () => {
  const childJson = canonicalStringify(compile(CHILD, { filename: "child.ts" }));
  await assertConformance({
    source: INVOKE,
    effects: { child_work: 7 },
    childArtifacts: { child: childJson },
    expectedResult: { t: "number", v: 7 },
    expectedEffectLog: ["child_work"],
    crashAts: ["after_create_child", "after_wait_checkpoint"],
  });
});
