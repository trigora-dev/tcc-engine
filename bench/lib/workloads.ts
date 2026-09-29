// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

import { canonicalStringify } from "../../frontends/typescript/src/canonical.ts";
import { compile } from "../../frontends/typescript/src/compile.ts";

export type WorkloadId = "A" | "B" | "C" | "D";

export type Workload = {
  id: WorkloadId;
  label: string;
  shape: string;
  source: string;
  effects: Record<string, unknown>;
  artifactJson: string;
};

const SOURCES: Record<WorkloadId, { label: string; shape: string; source: string; effects: Record<string, unknown> }> = {
  A: {
    label: "small-few",
    shape: "1 effect-result local, no reassignment",
    source: `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const result = await effect("generate", async () => generateSomething());
  const approval = await waitForEvent("approved");
  return { result, approval };
}
`,
    effects: { generate: 42 },
  },
  B: {
    label: "large-few",
    shape: "8 effect-result locals, no reassignment",
    source: `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  const l0 = await effect("l0", async () => 0);
  const l1 = await effect("l1", async () => 1);
  const l2 = await effect("l2", async () => 2);
  const l3 = await effect("l3", async () => 3);
  const l4 = await effect("l4", async () => 4);
  const l5 = await effect("l5", async () => 5);
  const l6 = await effect("l6", async () => 6);
  const l7 = await effect("l7", async () => 7);
  const approval = await waitForEvent("approved");
  return { l0, l1, l2, l3, l4, l5, l6, l7, approval };
}
`,
    effects: { l0: 0, l1: 1, l2: 2, l3: 3, l4: 4, l5: 5, l6: 6, l7: 7 },
  },
  C: {
    label: "large-many",
    shape: "4 effect-result locals, each reassigned once",
    source: `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  let a = await effect("a", async () => 1);
  let b = await effect("b", async () => 2);
  let c = await effect("c", async () => 3);
  let d = await effect("d", async () => 4);
  a = await effect("a2", async () => 11);
  b = await effect("b2", async () => 12);
  c = await effect("c2", async () => 13);
  d = await effect("d2", async () => 14);
  const approval = await waitForEvent("approved");
  return { a, b, c, d, approval };
}
`,
    effects: { a: 1, b: 2, c: 3, d: 4, a2: 11, b2: 12, c2: 13, d2: 14 },
  },
  D: {
    label: "small-many",
    shape: "1 effect-result local, reassigned 3 times",
    source: `
import { effect, waitForEvent } from "@trigora/sdk";
export default async function run() {
  let x = await effect("x1", async () => 1);
  x = await effect("x2", async () => 2);
  x = await effect("x3", async () => 3);
  x = await effect("x4", async () => 4);
  const approval = await waitForEvent("approved");
  return { x, approval };
}
`,
    effects: { x1: 1, x2: 2, x3: 3, x4: 4 },
  },
};

export function loadWorkloads(ids: WorkloadId[] = ["A", "B", "C", "D"]): Workload[] {
  return ids.map((id) => {
    const def = SOURCES[id];
    return {
      id,
      label: def.label,
      shape: def.shape,
      source: def.source,
      effects: def.effects,
      artifactJson: canonicalStringify(compile(def.source, { filename: `workload-${id}.ts` })),
    };
  });
}

export function dummyContinuation(executionId: string, revision: number, locals: number): string {
  return JSON.stringify({
    artifact_hash: "bench-hash",
    engine_format_version: 1,
    execution_id: executionId,
    frames: [
      {
        func_id: 0,
        locals: Array.from({ length: locals }, (_, i) => ({ t: "number", v: i })),
        pc: revision,
      },
    ],
    language_semantics_version: "ts.subset.v1",
    pending: null,
    result: null,
    revision,
    stack: [],
    status: "runnable",
    try_stack: [],
  });
}

export function dummyDelta(pc: number): string {
  return JSON.stringify({ frames: [{ index: 0, pc }] });
}
