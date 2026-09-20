import assert from "node:assert/strict";
import { mkdtempSync, readdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { canonicalStringify } from "../../../frontends/typescript/src/canonical.ts";
import { compile } from "../../../frontends/typescript/src/compile.ts";
import { applyDelta, assertConformance, resumeExecution, startExecution, Store } from "./kit.ts";
import type { ContinuationDelta, ContinuationJson } from "./reconstruct.ts";

const root = path.resolve(fileURLToPath(new URL("../../..", import.meta.url)));
const fixtures = path.join(root, "spec/fixtures/persist");
const cases = JSON.parse(readFileSync(path.join(root, "conformance/cases.json"), "utf8")) as {
  reconstruct: string[];
  semantic: Array<{ id: string; crash_ats?: string[] }>;
};

const FIRST = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const result = await effect("generate", async () => generateSomething());
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`;

const OTHER = `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const result = await effect("generate", async () => 99);
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`;

const wasmPath = path.join(root, "target/wasm32-unknown-unknown/release/tcc_wasm.wasm");

test("host conformance v1 reconstruct goldens", () => {
  const listed = new Set(cases.reconstruct.map((rel) => path.basename(rel)));
  const files = readdirSync(fixtures).filter((name) => name.endsWith(".json"));
  assert.deepEqual(new Set(files), listed);
  for (const rel of cases.reconstruct) {
    const fixture = JSON.parse(readFileSync(path.join(root, rel), "utf8")) as {
      base: ContinuationJson;
      delta: ContinuationDelta;
      expected: ContinuationJson;
    };
    const applied = applyDelta(fixture.base, fixture.delta);
    applied.revision = fixture.expected.revision;
    assert.deepEqual(applied, fixture.expected, rel);
  }
});

test("host conformance v1 first-example crash/resume", async () => {
  const first = cases.semantic.find((item) => item.id === "first-example");
  assert.ok(first?.crash_ats);
  await assertConformance({
    source: FIRST,
    filename: "first.ts",
    expectedResult: {
      t: "object",
      v: {
        approval: { t: "string", v: "ok" },
        result: { t: "number", v: 42 },
      },
    },
    crashAts: first.crash_ats,
  });
});

test("host conformance v1 artifact pinning", async () => {
  const dbPath = path.join(mkdtempSync(path.join(tmpdir(), "tcc-kit-")), "tcc.db");
  const stored = canonicalStringify(compile(FIRST, { filename: "first.ts" }));
  const other = canonicalStringify(compile(OTHER, { filename: "other.ts" }));
  assert.notEqual(
    JSON.parse(stored).envelope.artifact_hash,
    JSON.parse(other).envelope.artifact_hash,
  );
  const started = await startExecution({
    dbPath,
    wasmPath,
    artifactJson: stored,
    autoDeliverEvent: false,
  });
  assert.equal(started.status, "suspended");
  const store = new Store(dbPath);
  const execution = store.getExecution("first");
  const saved = store.getContinuation("first");
  store.close();
  assert.equal(JSON.parse(stored).envelope.artifact_hash, execution?.artifact_hash);
  assert.equal(JSON.parse(saved!.json).artifact_hash, execution?.artifact_hash);
  await assert.rejects(
    () =>
      resumeExecution({
        dbPath,
        wasmPath,
        artifactJson: other,
        eventPayload: "ok",
      }),
    /artifact/i,
  );
  const resumed = await resumeExecution({
    dbPath,
    wasmPath,
    eventPayload: "ok",
  });
  assert.equal(resumed.status, "completed");
});

test("host conformance v1 generated crash/resume", async () => {
  await assertConformance({
    source: `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const flag = await effect("generate", async () => 1);
  if (flag) {
    const approval = await waitForEvent("approved");
    return approval;
  }
  return 0;
}
`,
    filename: "branch.ts",
    expectedResult: { t: "string", v: "ok" },
    crashAts: ["after_persist_checkpoint:1", "after_wait_checkpoint"],
  });
});
