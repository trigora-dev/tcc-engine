// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

/**
 * Matched recovery: TCC continuation reconstruct vs history-replay prefix replay.
 *
 * Replay baseline (persist: "replay"): append `history` WAL rows and the effect
 * journal; on recovery, start the engine and replay the committed prefix.
 * Journal hits skip the effect provider. Same SQLite/WASM as TCC.
 *
 * Live size is measured continuation JSON bytes after a journaled string blob,
 * not a local-count proxy. History grows by reassigning one scratch local.
 */
import { canonicalStringify } from "../frontends/typescript/src/canonical.ts";
import { compile } from "../frontends/typescript/src/compile.ts";
import {
  resumeExecution,
  startExecution,
  type EffectRunner,
} from "../hosts/node/src/host.ts";
import { Store, type PersistMode } from "../hosts/node/src/store.ts";
import { buildMeta, printReport } from "./lib/report.ts";
import { benchScale } from "./lib/scale.ts";
import {
  latencyStats,
  relativeOverhead,
  throughputPerSec,
} from "./lib/stats.ts";
import { dbFile, wasmPath } from "./lib/store_metrics.ts";

type ReplayStorage = Extract<PersistMode, "replay" | "optimized">;

type ReplayRow = {
  scenario: "recovery_history_replay_baseline";
  storage: ReplayStorage;
  historyDepth: number;
  liveStateTargetBytes: number;
  liveStateBytes: number;
  recoversPerSec: number;
  overheadVsReplay?: number | null;
  [key: string]: unknown;
};

const LIVE_BAND = 0.15;
const EXECUTION_ID = "target";

function recoverySource(historyDepth: number): string {
  if (historyDepth < 2) {
    throw new Error(
      "historyDepth must include the live blob and at least one tick",
    );
  }
  const ticks = historyDepth - 1;
  const lines = [`  let tick = await effect("t0", async () => 0);`];
  for (let i = 1; i < ticks; i++) {
    lines.push(`  tick = await effect("t${i}", async () => 0);`);
  }
  return `
import { effect, waitForEvent } from "@tcc-engine/primitives";
export default async function run() {
  const live = await effect("live", async () => "");
${lines.join("\n")}
  const approval = await waitForEvent("approved");
  return { live, tick, approval };
}
`;
}

function effectsFor(
  historyDepth: number,
  blob: string,
): Record<string, unknown> {
  const effects: Record<string, unknown> = { live: blob };
  for (let i = 0; i < historyDepth - 1; i++) {
    effects[`t${i}`] = i;
  }
  return effects;
}

function blobOf(length: number): string {
  return "x".repeat(Math.max(0, length));
}

function inBand(actual: number, target: number): boolean {
  return Math.abs(actual - target) <= target * LIVE_BAND;
}

function forbidProvider(): EffectRunner {
  return (key) => {
    throw new Error(`replay recovery invoked provider for \`${key}\``);
  };
}

function liveStringLength(continuationJson: string): number {
  const parsed = JSON.parse(continuationJson) as {
    frames?: Array<{ locals?: Array<{ t?: string; v?: unknown }> }>;
    status?: string;
    pending?: unknown;
  };
  const locals = parsed.frames?.[0]?.locals ?? [];
  const live = locals.find(
    (slot) => slot.t === "string" && typeof slot.v === "string",
  );
  if (!live || typeof live.v !== "string") {
    throw new Error("missing live string local");
  }
  return live.v.length;
}

function rowKey(
  row: Pick<ReplayRow, "historyDepth" | "liveStateTargetBytes">,
): string {
  return `${row.historyDepth}:${row.liveStateTargetBytes}`;
}

function attachOverhead(rows: ReplayRow[]): void {
  const replay = new Map<string, number>();
  for (const row of rows) {
    if (row.storage === "replay") {
      replay.set(rowKey(row), Number(row.medianMs));
    }
  }
  for (const row of rows) {
    if (row.storage !== "optimized") {
      continue;
    }
    row.overheadVsReplay = relativeOverhead(
      Number(row.medianMs),
      replay.get(rowKey(row)) ?? 0,
    );
  }
}

async function populate(input: {
  dbPath: string;
  wasmPath: string;
  artifactJson: string;
  persist: ReplayStorage;
  effects: Record<string, unknown>;
}): Promise<{
  status: string;
  continuationJson: string | null;
  revision: number;
}> {
  const result = await startExecution({
    dbPath: input.dbPath,
    wasmPath: input.wasmPath,
    artifactJson: input.artifactJson,
    persist: input.persist,
    executionId: EXECUTION_ID,
    autoDeliverEvent: false,
    effects: input.effects,
  });
  if (result.status !== "suspended") {
    throw new Error(`expected suspended populate, got ${result.status}`);
  }
  return result;
}

function readTccContinuation(dbPath: string): {
  json: string;
  revision: number;
  plan: ReturnType<Store["recoverPlan"]>;
  wal: number;
  walBytes: number;
} {
  const store = new Store(dbPath, "optimized");
  try {
    const saved = store.getContinuation(EXECUTION_ID);
    if (!saved) {
      throw new Error("missing TCC continuation");
    }
    return {
      json: saved.json,
      revision: saved.revision,
      plan: store.recoverPlan(EXECUTION_ID),
      wal: store.walRecords(EXECUTION_ID).length,
      walBytes: store.retainedWalBytes(EXECUTION_ID),
    };
  } finally {
    store.close();
  }
}

function replayMeta(dbPath: string): {
  wal: number;
  walBytes: number;
  revision: number;
  liveLength: number;
  waitPending: boolean;
  status: string;
} {
  const store = new Store(dbPath, "replay");
  try {
    const execution = store.getExecution(EXECUTION_ID);
    if (!execution) {
      throw new Error("missing replay execution");
    }
    if (store.getContinuation(EXECUTION_ID)) {
      throw new Error("replay store unexpectedly has a continuation");
    }
    const effect = store.getEffect(EXECUTION_ID, "live");
    if (!effect?.result_json) {
      throw new Error("missing journaled live blob");
    }
    const parsed = JSON.parse(effect.result_json) as {
      t?: string;
      v?: unknown;
    };
    if (parsed.t !== "string" || typeof parsed.v !== "string") {
      throw new Error("journaled live blob is not a string");
    }
    return {
      wal: store.walRecords(EXECUTION_ID).length,
      walBytes: store.retainedWalBytes(EXECUTION_ID),
      revision: execution.revision,
      liveLength: parsed.v.length,
      waitPending: store.pendingWait(EXECUTION_ID)?.status === "pending",
      status: execution.status,
    };
  } finally {
    store.close();
  }
}

async function calibrateBlob(input: {
  wasmPath: string;
  artifactJson: string;
  historyDepth: number;
  targetBytes: number;
}): Promise<{ blob: string; liveStateBytes: number }> {
  let blobLen = Math.max(32, input.targetBytes - 800);
  let liveStateBytes = 0;
  for (let attempt = 0; attempt < 4; attempt++) {
    const blob = blobOf(blobLen);
    const dbPath = dbFile("tcc-calib-");
    await populate({
      dbPath,
      wasmPath: input.wasmPath,
      artifactJson: input.artifactJson,
      persist: "optimized",
      effects: effectsFor(input.historyDepth, blob),
    });
    liveStateBytes = readTccContinuation(dbPath).json.length;
    if (inBand(liveStateBytes, input.targetBytes)) {
      return { blob, liveStateBytes };
    }
    blobLen = Math.max(32, blobLen + (input.targetBytes - liveStateBytes));
  }
  throw new Error(
    `could not land live state near ${input.targetBytes} bytes (last ${liveStateBytes})`,
  );
}

export async function runRecoveryHistoryReplayBaseline(): Promise<unknown[]> {
  const scale = benchScale();
  const wasm = wasmPath();
  const meta = buildMeta({ host: "node", wasmPath: wasm, scale: scale.name });
  const rows: ReplayRow[] = [];

  for (const historyDepth of scale.recoveryReplayDepths) {
    const artifactJson = canonicalStringify(
      compile(recoverySource(historyDepth), {
        filename: `recovery-replay-${historyDepth}.ts`,
      }),
    );

    for (const liveStateTargetBytes of scale.recoveryReplayLiveBytes) {
      const { blob } = await calibrateBlob({
        wasmPath: wasm,
        artifactJson,
        historyDepth,
        targetBytes: liveStateTargetBytes,
      });
      const effects = effectsFor(historyDepth, blob);
      const tccDb = dbFile("tcc-rec-opt-");
      const replayDb = dbFile("tcc-rec-rep-");
      await populate({
        dbPath: tccDb,
        wasmPath: wasm,
        artifactJson,
        persist: "optimized",
        effects,
      });
      await populate({
        dbPath: replayDb,
        wasmPath: wasm,
        artifactJson,
        persist: "replay",
        effects,
      });

      const tcc = readTccContinuation(tccDb);
      if (!inBand(tcc.json.length, liveStateTargetBytes)) {
        throw new Error(
          `live state ${tcc.json.length} outside ${liveStateTargetBytes} ±${LIVE_BAND * 100}%`,
        );
      }
      const replayInfo = replayMeta(replayDb);
      const check = await resumeExecution({
        dbPath: replayDb,
        wasmPath: wasm,
        persist: "replay",
        executionId: EXECUTION_ID,
        autoDeliverEvent: false,
        runEffect: forbidProvider(),
      });
      if (check.status !== "suspended") {
        throw new Error(`replay recover status ${check.status}`);
      }
      const tccParsed = JSON.parse(tcc.json) as {
        status: string;
        pending: unknown;
      };
      if (tccParsed.status !== "waiting" && tccParsed.status !== "suspended") {
        throw new Error(`TCC continuation status ${tccParsed.status}`);
      }
      if (tccParsed.pending == null) {
        throw new Error("expected pending wait on TCC continuation");
      }
      if (liveStringLength(tcc.json) !== replayInfo.liveLength) {
        throw new Error(
          "TCC continuation live blob diverged from replay journal",
        );
      }
      const afterReplay = replayMeta(replayDb);
      if (!afterReplay.waitPending || afterReplay.status !== "suspended") {
        throw new Error("replay recover did not restore the wait");
      }

      const tccStore = new Store(tccDb, "optimized");
      try {
        const samplesMs: number[] = [];
        for (let i = 0; i < scale.recoverSamples; i++) {
          const t0 = performance.now();
          const saved = tccStore.getContinuation(EXECUTION_ID);
          samplesMs.push(performance.now() - t0);
          if (!saved) {
            throw new Error("missing continuation during TCC samples");
          }
        }
        const latency = latencyStats(samplesMs);
        rows.push({
          scenario: "recovery_history_replay_baseline",
          storage: "optimized",
          historyDepth,
          liveStateTargetBytes,
          liveStateBytes: tcc.json.length,
          revisions: tcc.revision,
          historyLength: tcc.wal,
          retainedWalBytes: tcc.walBytes,
          suffixLength: tcc.plan?.suffixLength ?? 0,
          snapshotRevision: tcc.plan?.snapshotRevision ?? 0,
          ...latency,
          recoversPerSec: throughputPerSec(
            scale.recoverSamples,
            samplesMs.reduce((a, b) => a + b, 0),
          ),
        });
      } finally {
        tccStore.close();
      }

      const replaySamples: number[] = [];
      for (let i = 0; i < scale.recoverSamples; i++) {
        const t0 = performance.now();
        const recovered = await resumeExecution({
          dbPath: replayDb,
          wasmPath: wasm,
          persist: "replay",
          executionId: EXECUTION_ID,
          autoDeliverEvent: false,
          runEffect: forbidProvider(),
        });
        replaySamples.push(performance.now() - t0);
        if (recovered.status !== "suspended") {
          throw new Error(`replay sample status ${recovered.status}`);
        }
      }
      const replayLatency = latencyStats(replaySamples);
      rows.push({
        scenario: "recovery_history_replay_baseline",
        storage: "replay",
        historyDepth,
        liveStateTargetBytes,
        liveStateBytes: tcc.json.length,
        revisions: replayInfo.revision,
        historyLength: replayInfo.wal,
        retainedWalBytes: replayInfo.walBytes,
        ...replayLatency,
        recoversPerSec: throughputPerSec(
          scale.recoverSamples,
          replaySamples.reduce((a, b) => a + b, 0),
        ),
      });
    }
  }

  attachOverhead(rows);
  printReport("recovery_history_replay_baseline", meta, rows);
  return rows;
}
