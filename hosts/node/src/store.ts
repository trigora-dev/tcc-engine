import { DatabaseSync } from "node:sqlite";

import { emitHostEvent, type HostEventInput, type HostObserver } from "./observe.ts";
import {
  applyDelta,
  persistModeFromEnv,
  reconstructContinuation,
  choosePackedKind,
  packingFromEnv,
  packingThresholdsFromEnv,
  MATERIALIZE_EVERY,
  NODE_ADAPTIVE_THRESHOLDS,
  NODE_PACKING_DEFAULT,
  type ContinuationDelta,
  type ContinuationJson,
  type PackingThresholds,
  type PersistKind,
  type PersistMode,
  type PersistPacking,
} from "./reconstruct.ts";

export {
  applyDelta,
  MATERIALIZE_EVERY,
  persistModeFromEnv,
  reconstructContinuation,
  choosePackedKind,
  packingFromEnv,
  packingThresholdsFromEnv,
  NODE_ADAPTIVE_THRESHOLDS,
  NODE_PACKING_DEFAULT,
};
export type {
  ContinuationDelta,
  ContinuationJson,
  PackingThresholds,
  PersistKind,
  PersistMode,
  PersistPacking,
};
export type { HostEvent, HostEventType, HostObserver } from "./observe.ts";

export type CheckpointCommit = {
  executionId: string;
  revision: number;
  json: string;
  status: string;
  ownerToken: string;
  kind?: PersistKind;
  deltaJson?: string | null;
  materialize?: boolean;
};

export type CreateChildOp = {
  invokeId: string;
  parentExecutionId: string;
  childExecutionId: string;
  flowName: string;
  artifactHash: string;
  ownerToken: string;
  leaseUntil: number;
  /** SQL NULL for `[]`. Otherwise a JSON array of tagged values. */
  argsJson: string | null;
};

export type CompleteChildOp = {
  invokeId: string;
  resultJson: string;
};

export type DurabilityOp =
  | ({ type: "checkpoint" } & CheckpointCommit)
  | ({ type: "createChild" } & CreateChildOp)
  | ({ type: "completeChild" } & CompleteChildOp);

export type ChildRow = {
  invoke_id: string;
  parent_execution_id: string;
  child_execution_id: string;
  flow_name: string;
  status: string;
  result_json: string | null;
  args_json: string | null;
};

export type EffectRow = {
  execution_id: string;
  key: string;
  idempotency_key: string;
  status: string;
  result_json: string | null;
};

export type ExecutionRow = {
  id: string;
  artifact_hash: string;
  revision: number;
  status: string;
  owner_token: string | null;
  lease_until: number | null;
};

export type WaitRow = {
  wait_id: string;
  execution_id: string;
  event_name: string;
  status: string;
};

export type StoreMetrics = {
  bytesWritten: number;
  snapshotCount: number;
  deltaCount: number;
  commitCount: number;
  batchOccupancySamples: number[];
  /** Sum of payload bytes written as deltas (optimized path). */
  deltaBytesWritten: number;
};

export type StoreProfile = { sqlApplyMs: number; commitMs: number };

export type StoreOptions = {
  packing?: PersistPacking;
  packingThresholds?: PackingThresholds;
};

export class Store {
  readonly db: DatabaseSync;
  readonly persist: PersistMode;
  readonly packing: PersistPacking;
  readonly packingThresholds: PackingThresholds;
  private readonly observer: HostObserver | undefined;
  private groupHeld = 0;
  private pending: DurabilityOp[] = [];
  private createdChildren: CreateChildOp[] = [];
  private queuedEvents: HostEventInput[] = [];
  private metricBytesWritten = 0;
  private metricSnapshotCount = 0;
  private metricDeltaCount = 0;
  private metricCommitCount = 0;
  private metricDeltaBytesWritten = 0;
  private metricBatchOccupancySamples: number[] = [];
  private profile: StoreProfile | undefined;
  private readonly statements = new Map<string, ReturnType<DatabaseSync["prepare"]>>();

  private statement(sql: string): ReturnType<DatabaseSync["prepare"]> {
    let prepared = this.statements.get(sql);
    if (!prepared) {
      prepared = this.db.prepare(sql);
      this.statements.set(sql, prepared);
    }
    return prepared;
  }

  enableProfile(): void {
    this.profile = { sqlApplyMs: 0, commitMs: 0 };
  }

  profileMetrics(): StoreProfile | undefined {
    return this.profile ? { ...this.profile } : undefined;
  }

  constructor(
    path: string,
    persist: PersistMode = persistModeFromEnv(),
    observer?: HostObserver,
    options?: StoreOptions,
  ) {
    this.persist = persist;
    this.observer = observer;
    this.packing = options?.packing ?? packingFromEnv(NODE_PACKING_DEFAULT);
    this.packingThresholds = options?.packingThresholds ?? packingThresholdsFromEnv(NODE_ADAPTIVE_THRESHOLDS);
    this.db = new DatabaseSync(path);
    this.db.exec("PRAGMA journal_mode = WAL");
    this.db.exec("PRAGMA foreign_keys = ON");
    this.db.exec("PRAGMA busy_timeout = 5000");
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS artifacts (
        hash TEXT PRIMARY KEY,
        json TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS executions (
        id TEXT PRIMARY KEY,
        artifact_hash TEXT NOT NULL,
        revision INTEGER NOT NULL DEFAULT 0,
        status TEXT NOT NULL,
        owner_token TEXT,
        lease_until INTEGER,
        FOREIGN KEY (artifact_hash) REFERENCES artifacts(hash)
      );
      CREATE TABLE IF NOT EXISTS continuations (
        execution_id TEXT PRIMARY KEY,
        revision INTEGER NOT NULL,
        json TEXT NOT NULL,
        FOREIGN KEY (execution_id) REFERENCES executions(id)
      );
      CREATE TABLE IF NOT EXISTS persist_wal (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        execution_id TEXT NOT NULL,
        revision INTEGER NOT NULL,
        kind TEXT NOT NULL,
        payload TEXT NOT NULL,
        UNIQUE(execution_id, revision)
      );
      CREATE TABLE IF NOT EXISTS exec_head (
        execution_id TEXT PRIMARY KEY,
        revision INTEGER NOT NULL,
        snapshot_revision INTEGER NOT NULL,
        wal_seq INTEGER NOT NULL,
        status TEXT NOT NULL,
        FOREIGN KEY (execution_id) REFERENCES executions(id)
      );
      CREATE TABLE IF NOT EXISTS effects (
        execution_id TEXT NOT NULL,
        key TEXT NOT NULL,
        idempotency_key TEXT NOT NULL,
        status TEXT NOT NULL,
        result_json TEXT,
        PRIMARY KEY (execution_id, key)
      );
      CREATE TABLE IF NOT EXISTS waits (
        wait_id TEXT PRIMARY KEY,
        execution_id TEXT NOT NULL,
        event_name TEXT NOT NULL,
        status TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        execution_id TEXT NOT NULL,
        name TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        consumed INTEGER NOT NULL DEFAULT 0
      );
      CREATE TABLE IF NOT EXISTS timers (
        execution_id TEXT NOT NULL,
        branch_id TEXT NOT NULL DEFAULT '',
        resume_at_ms INTEGER NOT NULL,
        status TEXT NOT NULL,
        PRIMARY KEY (execution_id, branch_id)
      );
      CREATE TABLE IF NOT EXISTS children (
        invoke_id TEXT PRIMARY KEY,
        parent_execution_id TEXT NOT NULL,
        child_execution_id TEXT NOT NULL,
        flow_name TEXT NOT NULL,
        status TEXT NOT NULL,
        result_json TEXT,
        args_json TEXT
      );
    `);
  }

  close(): void {
    this.db.close();
  }

  observe(event: HostEventInput): void {
    emitHostEvent(this.observer, event);
  }

  resetMetrics(): void {
    this.metricBytesWritten = 0;
    this.metricSnapshotCount = 0;
    this.metricDeltaCount = 0;
    this.metricCommitCount = 0;
    this.metricDeltaBytesWritten = 0;
    this.metricBatchOccupancySamples = [];
  }

  metrics(): StoreMetrics {
    return {
      bytesWritten: this.metricBytesWritten,
      snapshotCount: this.metricSnapshotCount,
      deltaCount: this.metricDeltaCount,
      commitCount: this.metricCommitCount,
      batchOccupancySamples: this.metricBatchOccupancySamples.slice(),
      deltaBytesWritten: this.metricDeltaBytesWritten,
    };
  }

  putArtifact(hash: string, json: string): void {
    this.db.prepare("INSERT OR IGNORE INTO artifacts(hash, json) VALUES (?, ?)").run(hash, json);
  }

  getArtifact(hash: string): string | undefined {
    const row = this.db.prepare("SELECT json FROM artifacts WHERE hash = ?").get(hash) as
      | { json: string }
      | undefined;
    return row?.json;
  }

  createExecution(id: string, artifactHash: string, ownerToken: string, leaseUntil: number): void {
    this.db
      .prepare(
        "INSERT INTO executions(id, artifact_hash, revision, status, owner_token, lease_until) VALUES (?, ?, 0, 'runnable', ?, ?)",
      )
      .run(id, artifactHash, ownerToken, leaseUntil);
  }

  getExecution(id: string): ExecutionRow | undefined {
    return this.statement("SELECT * FROM executions WHERE id = ?").get(id) as ExecutionRow | undefined;
  }

  takeLease(id: string, ownerToken: string, leaseUntil: number, now: number): void {
    const result = this.db
      .prepare(
        `UPDATE executions
         SET owner_token = ?, lease_until = ?
         WHERE id = ? AND (owner_token IS NULL OR owner_token = ? OR lease_until IS NULL OR lease_until < ?)`,
      )
      .run(ownerToken, leaseUntil, id, ownerToken, now);
    if (result.changes !== 1) {
      throw new Error(`execution \`${id}\` is owned by another worker`);
    }
  }

  getContinuation(
    executionId: string,
    options: { observeRestore?: boolean } = {},
  ): { revision: number; json: string } | undefined {
    if (this.persist === "replay") {
      return undefined;
    }
    if (this.persist === "optimized") {
      const reconstructed = this.reconstruct(executionId, options.observeRestore === true);
      if (reconstructed) {
        return reconstructed;
      }
    }
    const started = options.observeRestore ? performance.now() : 0;
    const saved = this.db
      .prepare("SELECT revision, json FROM continuations WHERE execution_id = ?")
      .get(executionId) as { revision: number; json: string } | undefined;
    if (saved && options.observeRestore) {
      this.observe({
        type: "continuation.restored",
        executionId,
        revision: saved.revision,
        durationMs: Number((performance.now() - started).toFixed(4)),
      });
    }
    return saved;
  }

  beginGroup(): void {
    this.groupHeld += 1;
  }

  isGrouping(): boolean {
    return this.groupHeld > 0;
  }

  abortGroup(): void {
    this.groupHeld = 0;
    this.pending = [];
    this.createdChildren = [];
  }

  takeCreatedChildren(): CreateChildOp[] {
    const created = this.createdChildren;
    this.createdChildren = [];
    return created;
  }

  endGroup(): void {
    if (this.groupHeld === 0) {
      return;
    }
    this.groupHeld -= 1;
    if (this.groupHeld === 0) {
      this.flushGroup();
    }
  }

  flushGroup(): void {
    if (this.pending.length === 0) {
      return;
    }
    const batch = this.pending;
    this.pending = [];
    this.queuedEvents = [];
    const before = [this.metricBytesWritten, this.metricSnapshotCount, this.metricDeltaCount, this.metricDeltaBytesWritten];
    const started = performance.now();
    this.db.exec("BEGIN IMMEDIATE");
    try {
      const applyStart = this.profile ? performance.now() : 0;
      const created: CreateChildOp[] = [];
      for (const op of batch) {
        if (op.type === "createChild") {
          this.applyCreateChild(op);
          created.push(op);
        } else if (op.type === "completeChild") {
          this.applyCompleteChild(op);
        } else {
          this.applyCheckpoint(op);
        }
      }
      if (this.profile) this.profile.sqlApplyMs += performance.now() - applyStart;
      const commitStart = this.profile ? performance.now() : 0;
      this.db.exec("COMMIT");
      if (this.profile) this.profile.commitMs += performance.now() - commitStart;
      this.metricCommitCount += 1;
      this.metricBatchOccupancySamples.push(batch.filter((op) => op.type === "checkpoint").length);
      this.createdChildren.push(...created);
      const queued = this.queuedEvents;
      this.queuedEvents = [];
      for (const event of queued) {
        this.observe(event);
      }
      this.observe({
        type: "batch.committed",
        batchSize: batch.length,
        durationMs: Number((performance.now() - started).toFixed(4)),
      });
    } catch (err) {
      this.db.exec("ROLLBACK");
      this.queuedEvents = [];
      [this.metricBytesWritten, this.metricSnapshotCount, this.metricDeltaCount, this.metricDeltaBytesWritten] = before;
      this.observe({
        type: "runtime.error",
        message: err instanceof Error ? err.message : String(err),
      });
      throw err;
    }
  }

  enqueueCheckpoint(
    executionId: string,
    revision: number,
    json: string,
    status: string,
    ownerToken: string,
    extras: {
      kind?: PersistKind;
      deltaJson?: string | null;
      materialize?: boolean;
    } = {},
  ): void {
    this.pending.push({
      type: "checkpoint",
      executionId,
      revision,
      json,
      status,
      ownerToken,
      kind: extras.kind,
      deltaJson: extras.deltaJson,
      materialize: extras.materialize,
    });
  }

  enqueueCreateChild(op: CreateChildOp): void {
    this.pending.push({ type: "createChild", ...op });
  }

  enqueueCompleteChild(op: CompleteChildOp): void {
    this.pending.push({ type: "completeChild", ...op });
  }

  flushIfUngrouped(): void {
    if (this.groupHeld === 0) {
      this.flushGroup();
    }
  }

  commitCheckpoint(
    executionId: string,
    revision: number,
    json: string,
    status: string,
    ownerToken: string,
    extras: {
      kind?: PersistKind;
      deltaJson?: string | null;
      materialize?: boolean;
    } = {},
  ): void {
    this.enqueueCheckpoint(executionId, revision, json, status, ownerToken, extras);
    this.flushIfUngrouped();
  }

  execHead(executionId: string):
    | {
        execution_id: string;
        revision: number;
        snapshot_revision: number;
        wal_seq: number;
        status: string;
      }
    | undefined {
    return this.statement("SELECT * FROM exec_head WHERE execution_id = ?").get(executionId) as
      | {
          execution_id: string;
          revision: number;
          snapshot_revision: number;
          wal_seq: number;
          status: string;
        }
      | undefined;
  }

  walRecords(executionId: string): Array<{
    seq: number;
    revision: number;
    kind: string;
    payload: string;
  }> {
    return this.db
      .prepare(
        "SELECT seq, revision, kind, payload FROM persist_wal WHERE execution_id = ? ORDER BY revision",
      )
      .all(executionId) as Array<{
      seq: number;
      revision: number;
      kind: string;
      payload: string;
    }>;
  }

  walRowCount(): number {
    const row = this.db.prepare("SELECT COUNT(*) AS n FROM persist_wal").get() as { n: number };
    return Number(row.n);
  }

  recoverPlan(executionId: string): {
    snapshotRevision: number;
    suffixLength: number;
    usedIndex: boolean;
    detail: string;
  } | undefined {
    const head = this.execHead(executionId);
    if (!head) {
      return undefined;
    }
    const plan = this.db
      .prepare(
        `EXPLAIN QUERY PLAN
         SELECT payload FROM persist_wal
         WHERE execution_id = ? AND revision > ? AND revision <= ?
         ORDER BY revision`,
      )
      .all(executionId, head.snapshot_revision, head.revision) as Array<{ detail?: string }>;
    const detail = plan.map((row) => String(row.detail ?? "")).join("\n");
    return {
      snapshotRevision: head.snapshot_revision,
      suffixLength: head.revision - head.snapshot_revision,
      usedIndex: /using (covering )?index/i.test(detail),
      detail,
    };
  }

  retainedWalBytes(executionId?: string): number {
    const row = executionId
      ? (this.db
          .prepare("SELECT COALESCE(SUM(LENGTH(payload)), 0) AS n FROM persist_wal WHERE execution_id = ?")
          .get(executionId) as { n: number })
      : (this.db.prepare("SELECT COALESCE(SUM(LENGTH(payload)), 0) AS n FROM persist_wal").get() as {
          n: number;
        });
    return Number(row.n);
  }

  private reconstruct(
    executionId: string,
    observeRestore = false,
  ): { revision: number; json: string } | undefined {
    const head = this.execHead(executionId);
    if (!head) {
      return undefined;
    }
    const started = observeRestore ? performance.now() : 0;
    const snapshot = this.db
      .prepare(
        "SELECT payload FROM persist_wal WHERE execution_id = ? AND revision = ? AND kind = 'snapshot'",
      )
      .get(executionId, head.snapshot_revision) as { payload: string } | undefined;
    if (!snapshot) {
      throw new Error(`missing snapshot revision ${head.snapshot_revision} for \`${executionId}\``);
    }
    const suffix = this.db
      .prepare(
        `SELECT payload FROM persist_wal
         WHERE execution_id = ? AND revision > ? AND revision <= ?
         ORDER BY revision`,
      )
      .all(executionId, head.snapshot_revision, head.revision) as Array<{ payload: string }>;
    if (suffix.length > MATERIALIZE_EVERY) {
      throw new Error(
        `WAL suffix for \`${executionId}\` exceeds materialization bound ${MATERIALIZE_EVERY}`,
      );
    }
    const reconstructed = reconstructContinuation(
      JSON.parse(snapshot.payload) as ContinuationJson,
      suffix.map((row) => JSON.parse(row.payload) as ContinuationDelta),
      head.revision,
    );
    if (observeRestore) {
      this.observe({
        type: "continuation.restored",
        executionId,
        revision: head.revision,
        durationMs: Number((performance.now() - started).toFixed(4)),
        suffixLength: suffix.length,
      });
    }
    return { revision: head.revision, json: JSON.stringify(reconstructed) };
  }

  private applyCheckpoint(write: CheckpointCommit): void {
    const current = this.getExecution(write.executionId);
    if (!current) {
      throw new Error(`unknown execution \`${write.executionId}\``);
    }
    if (current.owner_token !== write.ownerToken) {
      throw new Error(`execution \`${write.executionId}\` is owned by another worker`);
    }
    if (this.persist === "replay" && current.revision >= write.revision) {
      return;
    }
    if (current.revision !== write.revision - 1) {
      throw new Error(
        `revision conflict for \`${write.executionId}\`: expected ${write.revision - 1}, found ${current.revision}`,
      );
    }
    if (this.persist === "naive") {
      this.applyNaiveCheckpoint(write, current.revision);
    } else if (this.persist === "replay") {
      this.applyReplayCheckpoint(write);
    } else {
      this.applyOptimizedCheckpoint(write);
    }
    const execUpdated = this
      .statement(
        "UPDATE executions SET revision = ?, status = ? WHERE id = ? AND revision = ? AND owner_token = ?",
      )
      .run(write.revision, write.status, write.executionId, current.revision, write.ownerToken);
    if (execUpdated.changes !== 1) {
      throw new Error(`execution cas failed for \`${write.executionId}\``);
    }
  }

  private applyNaiveCheckpoint(write: CheckpointCommit, currentRevision: number): void {
    const bytes = Buffer.byteLength(write.json);
    this.metricBytesWritten += bytes;
    this.metricSnapshotCount += 1;
    this.queuedEvents.push(
      {
        type: "checkpoint.persisted",
        executionId: write.executionId,
        revision: write.revision,
        kind: "snapshot",
        bytes,
      },
      {
        type: "checkpoint.materialized",
        executionId: write.executionId,
        revision: write.revision,
        kind: "snapshot",
        bytes,
      },
    );
    const existing = this
      .statement("SELECT revision FROM continuations WHERE execution_id = ?")
      .get(write.executionId) as { revision: number } | undefined;
    if (!existing) {
      this
        .statement("INSERT INTO continuations(execution_id, revision, json) VALUES (?, ?, ?)")
        .run(write.executionId, write.revision, write.json);
      return;
    }
    const updated = this
      .statement(
        "UPDATE continuations SET revision = ?, json = ? WHERE execution_id = ? AND revision = ?",
      )
      .run(write.revision, write.json, write.executionId, currentRevision);
    if (updated.changes !== 1) {
      throw new Error(`continuation cas failed for \`${write.executionId}\``);
    }
  }

  private applyReplayCheckpoint(write: CheckpointCommit): void {
    const payload = JSON.stringify({ revision: write.revision, status: write.status });
    const bytes = Buffer.byteLength(payload);
    this.metricBytesWritten += bytes;
    this.queuedEvents.push({
      type: "checkpoint.persisted",
      executionId: write.executionId,
      revision: write.revision,
      bytes,
    });
    this.statement(
      "INSERT INTO persist_wal(execution_id, revision, kind, payload) VALUES (?, ?, 'history', ?)",
    ).run(write.executionId, write.revision, payload);
  }

  private applyOptimizedCheckpoint(write: CheckpointCommit): void {
    const head = this.execHead(write.executionId);
    const requestedKind: PersistKind = write.kind ?? "snapshot";
    const mustMaterialize =
      write.materialize === true ||
      requestedKind === "snapshot" ||
      (head !== undefined && write.revision - head.snapshot_revision > MATERIALIZE_EVERY);
    const fullBytes = Buffer.byteLength(write.json);
    const deltaBytes = write.deltaJson == null ? null : Buffer.byteLength(write.deltaJson);
    const kind: PersistKind = choosePackedKind({
      mustMaterialize,
      packing: this.packing,
      fullBytes,
      deltaBytes,
      thresholds: this.packingThresholds,
    });
    const payload = kind === "snapshot" ? write.json : (write.deltaJson ?? write.json);
    const bytes = Buffer.byteLength(payload);
    this.metricBytesWritten += bytes;
    if (kind === "snapshot") {
      this.metricSnapshotCount += 1;
    } else {
      this.metricDeltaCount += 1;
      this.metricDeltaBytesWritten += bytes;
    }
    this.queuedEvents.push({
      type: "checkpoint.persisted",
      executionId: write.executionId,
      revision: write.revision,
      kind,
      bytes,
    });
    if (kind === "snapshot") {
      this.queuedEvents.push({
        type: "checkpoint.materialized",
        executionId: write.executionId,
        revision: write.revision,
        kind: "snapshot",
        bytes,
      });
    }
    const inserted = this
      .statement(
        "INSERT INTO persist_wal(execution_id, revision, kind, payload) VALUES (?, ?, ?, ?)",
      )
      .run(write.executionId, write.revision, kind, payload);
    const snapshotRevision = kind === "snapshot" ? write.revision : head?.snapshot_revision;
    if (snapshotRevision === undefined) {
      throw new Error(`delta without snapshot for \`${write.executionId}\``);
    }
    this
      .statement(
        `INSERT INTO exec_head(execution_id, revision, snapshot_revision, wal_seq, status)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(execution_id) DO UPDATE SET
           revision = excluded.revision,
           snapshot_revision = excluded.snapshot_revision,
           wal_seq = excluded.wal_seq,
           status = excluded.status`,
      )
      .run(
        write.executionId,
        write.revision,
        snapshotRevision,
        inserted.lastInsertRowid,
        write.status,
      );
  }

  getEffect(executionId: string, key: string): EffectRow | undefined {
    return this.db
      .prepare("SELECT * FROM effects WHERE execution_id = ? AND key = ?")
      .get(executionId, key) as EffectRow | undefined;
  }

  markEffectStarted(executionId: string, key: string, idempotencyKey: string): void {
    this.db
      .prepare(
        `INSERT INTO effects(execution_id, key, idempotency_key, status, result_json)
         VALUES (?, ?, ?, 'started', NULL)
         ON CONFLICT(execution_id, key) DO UPDATE SET
           idempotency_key = excluded.idempotency_key
         WHERE effects.status != 'completed'`,
      )
      .run(executionId, key, idempotencyKey);
  }

  completeEffect(executionId: string, key: string, idempotencyKey: string, resultJson: string): void {
    this.db
      .prepare(
        `INSERT INTO effects(execution_id, key, idempotency_key, status, result_json)
         VALUES (?, ?, ?, 'completed', ?)
         ON CONFLICT(execution_id, key) DO UPDATE SET
           status = 'completed',
           idempotency_key = excluded.idempotency_key,
           result_json = excluded.result_json`,
      )
      .run(executionId, key, idempotencyKey, resultJson);
  }

  failEffect(executionId: string, key: string, idempotencyKey: string, resultJson: string): void {
    this.db
      .prepare(
        `INSERT INTO effects(execution_id, key, idempotency_key, status, result_json)
         VALUES (?, ?, ?, 'failed', ?)
         ON CONFLICT(execution_id, key) DO UPDATE SET
           status = 'failed',
           idempotency_key = excluded.idempotency_key,
           result_json = excluded.result_json
         WHERE effects.status != 'completed'`,
      )
      .run(executionId, key, idempotencyKey, resultJson);
  }

  upsertWait(waitId: string, executionId: string, eventName: string): void {
    this.db
      .prepare(
        `INSERT INTO waits(wait_id, execution_id, event_name, status)
         VALUES (?, ?, ?, 'pending')
         ON CONFLICT(wait_id) DO NOTHING`,
      )
      .run(waitId, executionId, eventName);
  }

  getWait(waitId: string): WaitRow | undefined {
    return this.db.prepare("SELECT * FROM waits WHERE wait_id = ?").get(waitId) as WaitRow | undefined;
  }

  pendingWait(executionId: string): WaitRow | undefined {
    return this.db
      .prepare(
        "SELECT * FROM waits WHERE execution_id = ? AND status = 'pending' ORDER BY rowid LIMIT 1",
      )
      .get(executionId) as WaitRow | undefined;
  }

  resolveWait(waitId: string): void {
    this.db.prepare("UPDATE waits SET status = 'resolved' WHERE wait_id = ?").run(waitId);
  }

  enqueueEvent(executionId: string, name: string, payloadJson: string): void {
    this.db
      .prepare("INSERT INTO events(execution_id, name, payload_json, consumed) VALUES (?, ?, ?, 0)")
      .run(executionId, name, payloadJson);
  }

  takeEvent(executionId: string, name: string): { id: number; payload_json: string } | undefined {
    const row = this.db
      .prepare(
        "SELECT id, payload_json FROM events WHERE execution_id = ? AND name = ? AND consumed = 0 ORDER BY id LIMIT 1",
      )
      .get(executionId, name) as { id: number; payload_json: string } | undefined;
    if (!row) {
      return undefined;
    }
    this.db.prepare("UPDATE events SET consumed = 1 WHERE id = ?").run(row.id);
    return row;
  }

  upsertTimer(executionId: string, resumeAtMs: number, branchId = ""): void {
    this.db
      .prepare(
        `INSERT INTO timers(execution_id, branch_id, resume_at_ms, status)
         VALUES (?, ?, ?, 'pending')
         ON CONFLICT(execution_id, branch_id) DO NOTHING`,
      )
      .run(executionId, branchId, resumeAtMs);
  }

  pendingTimer(executionId: string, branchId = ""): { branch_id: string; resume_at_ms: number; status: string } | undefined {
    return this.db
      .prepare(
        "SELECT branch_id, resume_at_ms, status FROM timers WHERE execution_id = ? AND branch_id = ? AND status = 'pending'",
      )
      .get(executionId, branchId) as { branch_id: string; resume_at_ms: number; status: string } | undefined;
  }

  resolveTimer(executionId: string, branchId = ""): void {
    this.db
      .prepare("UPDATE timers SET status = 'resolved' WHERE execution_id = ? AND branch_id = ?")
      .run(executionId, branchId);
  }

  private applyCreateChild(op: CreateChildOp): void {
    const argsJson = canonicalArgsJson(op.argsJson);
    const existing = this.getChild(op.invokeId);
    if (existing) {
      if (existing.args_json !== argsJson) {
        throw new Error(`child ${op.invokeId} argument vector mismatch`);
      }
      return;
    }
    this.statement(
      `INSERT INTO children(invoke_id, parent_execution_id, child_execution_id, flow_name, status, result_json, args_json)
       VALUES (?, ?, ?, ?, 'pending', NULL, ?)
       ON CONFLICT(invoke_id) DO NOTHING`,
    ).run(op.invokeId, op.parentExecutionId, op.childExecutionId, op.flowName, argsJson);
    const stored = this.getChild(op.invokeId);
    if (stored?.args_json !== argsJson) {
      throw new Error(`child ${op.invokeId} argument vector mismatch`);
    }
    this.statement(
      `INSERT OR IGNORE INTO executions(id, artifact_hash, revision, status, owner_token, lease_until)
       VALUES (?, ?, 0, 'runnable', ?, ?)`,
    ).run(op.childExecutionId, op.artifactHash, op.ownerToken, op.leaseUntil);
    this.queuedEvents.push({
      type: "child.created",
      executionId: op.childExecutionId,
    });
  }

  private applyCompleteChild(op: CompleteChildOp): void {
    const child = this.getChild(op.invokeId);
    this.statement(
      "UPDATE children SET status = 'completed', result_json = ? WHERE invoke_id = ?",
    ).run(op.resultJson, op.invokeId);
    this.queuedEvents.push({
      type: "child.completed",
      executionId: child?.child_execution_id,
    });
  }

  upsertChild(
    invokeId: string,
    parentExecutionId: string,
    childExecutionId: string,
    flowName: string,
  ): void {
    this.enqueueCreateChild({
      invokeId,
      parentExecutionId,
      childExecutionId,
      flowName,
      artifactHash: "",
      ownerToken: "",
      leaseUntil: 0,
      argsJson: null,
    });
    this.flushIfUngrouped();
  }

  getChild(invokeId: string): ChildRow | undefined {
    return this.statement("SELECT * FROM children WHERE invoke_id = ?").get(invokeId) as ChildRow | undefined;
  }

  getChildByExecutionId(childExecutionId: string): ChildRow | undefined {
    return this.statement("SELECT * FROM children WHERE child_execution_id = ?").get(childExecutionId) as
      | ChildRow
      | undefined;
  }

  completeChild(invokeId: string, resultJson: string): void {
    this.enqueueCompleteChild({ invokeId, resultJson });
    this.flushIfUngrouped();
  }
}

function canonicalArgsJson(text: string | null): string | null {
  if (text == null || text === "[]") return null;
  return text;
}
