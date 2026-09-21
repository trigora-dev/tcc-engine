import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import type { HostConformanceDriver } from "./driver.ts";
import type { ContinuationDelta, ContinuationJson } from "./reconstruct.ts";

export type { HostConformanceDriver } from "./driver.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");

type ProgramSpec = { file?: string; source?: string; filename?: string };

type SemanticCase = {
  id: string;
  kind?: "crash-resume" | "artifact-pinning";
  crash_ats?: string[];
  expected_result?: unknown;
  expected_effect_log?: string[];
  effects?: Record<string, unknown>;
  event_payload?: unknown;
  auto_deliver_event?: boolean;
  cancel?: boolean;
  typescript?: ProgramSpec | { stored: ProgramSpec; other: ProgramSpec };
  python?: ProgramSpec | { stored: ProgramSpec; other: ProgramSpec };
};

type CasesFile = {
  reconstruct: string[];
  semantic: SemanticCase[];
};

export function loadCases(): CasesFile {
  return JSON.parse(
    readFileSync(path.join(here, "cases.json"), "utf8"),
  ) as CasesFile;
}

function loadProgram(spec: ProgramSpec): { source: string; filename: string } {
  if (spec.file) {
    const abs = path.join(here, spec.file);
    return {
      source: readFileSync(abs, "utf8"),
      filename: spec.filename ?? path.basename(spec.file),
    };
  }
  if (spec.source === undefined) {
    throw new Error("program spec needs file or source");
  }
  return { source: spec.source, filename: spec.filename ?? "input.ts" };
}

function programFor(
  item: SemanticCase,
  language: "typescript" | "python",
): ProgramSpec | { stored: ProgramSpec; other: ProgramSpec } {
  const spec = language === "python" ? item.python : item.typescript;
  if (!spec) {
    throw new Error(`case ${item.id} has no ${language} program`);
  }
  return spec;
}

function isPinningSpec(
  spec: ProgramSpec | { stored: ProgramSpec; other: ProgramSpec },
): spec is {
  stored: ProgramSpec;
  other: ProgramSpec;
} {
  return "stored" in spec && "other" in spec;
}

async function runReconstruct(
  driver: HostConformanceDriver,
  cases: CasesFile,
): Promise<void> {
  const fixtures = path.join(root, "spec/fixtures/persist");
  const listed = new Set(cases.reconstruct.map((rel) => path.basename(rel)));
  const files = readdirSync(fixtures).filter((name) => name.endsWith(".json"));
  assert.deepEqual(new Set(files), listed, "reconstruct fixture listing");
  for (const rel of cases.reconstruct) {
    const fixture = JSON.parse(readFileSync(path.join(root, rel), "utf8")) as {
      base: ContinuationJson;
      delta: ContinuationDelta;
      expected: ContinuationJson;
    };
    const applied = driver.applyDelta(fixture.base, fixture.delta);
    applied.revision = fixture.expected.revision;
    assert.deepEqual(applied, fixture.expected, rel);
  }
}

async function runCrashResume(
  driver: HostConformanceDriver,
  item: SemanticCase,
): Promise<void> {
  const spec = programFor(item, driver.language);
  if (isPinningSpec(spec)) {
    throw new Error(`case ${item.id} is not a crash-resume program`);
  }
  const program = loadProgram(spec);
  const artifactJson = driver.compile(program.source, program.filename);
  const expectedStatus = item.cancel ? "cancelled" : "completed";
  const started = await driver.start({
    artifactJson,
    effects: item.effects,
    eventPayload: item.event_payload,
    autoDeliverEvent: item.auto_deliver_event,
    cancel: item.cancel,
  });
  assert.equal(
    started.status,
    expectedStatus,
    `${item.id} uninterrupted status`,
  );
  assert.deepEqual(
    started.result,
    item.expected_result,
    `${item.id} uninterrupted result`,
  );
  if (item.expected_effect_log) {
    assert.deepEqual(
      await driver.effectLog(started.handle),
      item.expected_effect_log,
      `${item.id} uninterrupted log`,
    );
  }
  for (const crashAt of item.crash_ats ?? []) {
    const handle = await driver.crashAt({
      artifactJson,
      crashAt,
      effects: item.effects,
      eventPayload: item.event_payload,
      autoDeliverEvent: item.auto_deliver_event,
      cancel: item.cancel,
    });
    const resumed = await driver.resume({
      handle,
      effects: item.effects,
      eventPayload: item.event_payload,
      autoDeliverEvent: item.auto_deliver_event,
      cancel: item.cancel,
    });
    assert.equal(
      resumed.status,
      expectedStatus,
      `${item.id} resume after ${crashAt} status`,
    );
    assert.deepEqual(
      resumed.result,
      item.expected_result,
      `${item.id} resume after ${crashAt} result`,
    );
    if (item.expected_effect_log) {
      assert.deepEqual(
        await driver.effectLog(handle),
        item.expected_effect_log,
        `${item.id} resume after ${crashAt} log`,
      );
    }
  }
}

async function runPinning(
  driver: HostConformanceDriver,
  item: SemanticCase,
): Promise<void> {
  const spec = programFor(item, driver.language);
  if (!isPinningSpec(spec)) {
    throw new Error(`case ${item.id} needs stored/other programs`);
  }
  const storedProgram = loadProgram(spec.stored);
  const otherProgram = loadProgram(spec.other);
  const stored = driver.compile(storedProgram.source, storedProgram.filename);
  const other = driver.compile(otherProgram.source, otherProgram.filename);
  const storedHash = (
    JSON.parse(stored) as { envelope: { artifact_hash: string } }
  ).envelope.artifact_hash;
  const otherHash = (
    JSON.parse(other) as { envelope: { artifact_hash: string } }
  ).envelope.artifact_hash;
  assert.notEqual(storedHash, otherHash, `${item.id} artifacts must differ`);
  const started = await driver.start({
    artifactJson: stored,
    autoDeliverEvent: false,
  });
  assert.equal(started.status, "suspended", `${item.id} start`);
  const continuation = await driver.readContinuation(started.handle);
  assert.equal(
    continuation.artifact_hash,
    storedHash,
    `${item.id} pinned hash`,
  );
  await assert.rejects(
    () =>
      driver.resume({
        handle: started.handle,
        artifactJson: other,
        eventPayload: "ok",
      }),
    /artifact/i,
    `${item.id} substitute must fail`,
  );
  const resumed = await driver.resume({
    handle: started.handle,
    eventPayload: "ok",
  });
  assert.equal(
    resumed.status,
    "completed",
    `${item.id} stored artifact resume`,
  );
}

/** Run host-conformance-v1 against a driver. Semantic only: no SQLite schema checks. */
export async function runKit(driver: HostConformanceDriver): Promise<void> {
  const cases = loadCases();
  await runReconstruct(driver, cases);
  for (const item of cases.semantic) {
    const kind =
      item.kind ??
      (item.id === "artifact-pinning" ? "artifact-pinning" : "crash-resume");
    if (kind === "artifact-pinning") {
      await runPinning(driver, item);
    } else {
      await runCrashResume(driver, item);
    }
  }
}
