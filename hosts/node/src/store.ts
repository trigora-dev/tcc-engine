import { DatabaseSync } from "node:sqlite";

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

export class Store {
  readonly db: DatabaseSync;

  constructor(path: string) {
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
        execution_id TEXT PRIMARY KEY,
        resume_at_ms INTEGER NOT NULL,
        status TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS children (
        invoke_id TEXT PRIMARY KEY,
        parent_execution_id TEXT NOT NULL,
        child_execution_id TEXT NOT NULL,
        flow_name TEXT NOT NULL,
        status TEXT NOT NULL,
        result_json TEXT
      );
    `);
  }

  close(): void {
    this.db.close();
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
    return this.db.prepare("SELECT * FROM executions WHERE id = ?").get(id) as ExecutionRow | undefined;
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

  getContinuation(executionId: string): { revision: number; json: string } | undefined {
    return this.db
      .prepare("SELECT revision, json FROM continuations WHERE execution_id = ?")
      .get(executionId) as { revision: number; json: string } | undefined;
  }

  commitCheckpoint(
    executionId: string,
    revision: number,
    json: string,
    status: string,
    ownerToken: string,
  ): void {
    this.db.exec("BEGIN IMMEDIATE");
    try {
      const current = this.getExecution(executionId);
      if (!current) {
        throw new Error(`unknown execution \`${executionId}\``);
      }
      if (current.owner_token !== ownerToken) {
        throw new Error(`execution \`${executionId}\` is owned by another worker`);
      }
      if (current.revision !== revision - 1) {
        throw new Error(
          `revision conflict for \`${executionId}\`: expected ${revision - 1}, found ${current.revision}`,
        );
      }
      const existing = this.getContinuation(executionId);
      if (!existing) {
        this.db
          .prepare("INSERT INTO continuations(execution_id, revision, json) VALUES (?, ?, ?)")
          .run(executionId, revision, json);
      } else {
        const updated = this.db
          .prepare(
            "UPDATE continuations SET revision = ?, json = ? WHERE execution_id = ? AND revision = ?",
          )
          .run(revision, json, executionId, current.revision);
        if (updated.changes !== 1) {
          throw new Error(`continuation cas failed for \`${executionId}\``);
        }
      }
      const execUpdated = this.db
        .prepare(
          "UPDATE executions SET revision = ?, status = ? WHERE id = ? AND revision = ? AND owner_token = ?",
        )
        .run(revision, status, executionId, current.revision, ownerToken);
      if (execUpdated.changes !== 1) {
        throw new Error(`execution cas failed for \`${executionId}\``);
      }
      this.db.exec("COMMIT");
    } catch (err) {
      this.db.exec("ROLLBACK");
      throw err;
    }
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
      .prepare("SELECT * FROM waits WHERE execution_id = ? AND status = 'pending' LIMIT 1")
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

  upsertTimer(executionId: string, resumeAtMs: number): void {
    this.db
      .prepare(
        `INSERT INTO timers(execution_id, resume_at_ms, status)
         VALUES (?, ?, 'pending')
         ON CONFLICT(execution_id) DO NOTHING`,
      )
      .run(executionId, resumeAtMs);
  }

  pendingTimer(executionId: string): { resume_at_ms: number; status: string } | undefined {
    return this.db
      .prepare("SELECT resume_at_ms, status FROM timers WHERE execution_id = ?")
      .get(executionId) as { resume_at_ms: number; status: string } | undefined;
  }

  resolveTimer(executionId: string): void {
    this.db.prepare("UPDATE timers SET status = 'resolved' WHERE execution_id = ?").run(executionId);
  }

  upsertChild(
    invokeId: string,
    parentExecutionId: string,
    childExecutionId: string,
    flowName: string,
  ): void {
    this.db
      .prepare(
        `INSERT INTO children(invoke_id, parent_execution_id, child_execution_id, flow_name, status, result_json)
         VALUES (?, ?, ?, ?, 'pending', NULL)
         ON CONFLICT(invoke_id) DO NOTHING`,
      )
      .run(invokeId, parentExecutionId, childExecutionId, flowName);
  }

  getChild(invokeId: string): {
    invoke_id: string;
    parent_execution_id: string;
    child_execution_id: string;
    flow_name: string;
    status: string;
    result_json: string | null;
  } | undefined {
    return this.db.prepare("SELECT * FROM children WHERE invoke_id = ?").get(invokeId) as
      | {
          invoke_id: string;
          parent_execution_id: string;
          child_execution_id: string;
          flow_name: string;
          status: string;
          result_json: string | null;
        }
      | undefined;
  }

  completeChild(invokeId: string, resultJson: string): void {
    this.db
      .prepare("UPDATE children SET status = 'completed', result_json = ? WHERE invoke_id = ?")
      .run(resultJson, invokeId);
  }
}
