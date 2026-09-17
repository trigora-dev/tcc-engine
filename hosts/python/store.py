"""SQLite persistence for the Python reference host."""

from __future__ import annotations

import sqlite3
from typing import Any


class Store:
    def __init__(self, path: str) -> None:
        self.db = sqlite3.connect(path)
        self.db.isolation_level = None
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode = WAL")
        self.db.execute("PRAGMA foreign_keys = ON")
        self.db.execute("PRAGMA busy_timeout = 5000")
        self.db.executescript(
            """
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
            """
        )

    def close(self) -> None:
        self.db.close()

    def put_artifact(self, hash_: str, json: str) -> None:
        self.db.execute("INSERT OR IGNORE INTO artifacts(hash, json) VALUES (?, ?)", (hash_, json))
        self.db.commit()

    def get_artifact(self, hash_: str) -> str | None:
        row = self.db.execute("SELECT json FROM artifacts WHERE hash = ?", (hash_,)).fetchone()
        return None if row is None else row["json"]

    def create_execution(self, id_: str, artifact_hash: str, owner_token: str, lease_until: int) -> None:
        self.db.execute(
            "INSERT INTO executions(id, artifact_hash, revision, status, owner_token, lease_until) VALUES (?, ?, 0, 'runnable', ?, ?)",
            (id_, artifact_hash, owner_token, lease_until),
        )
        self.db.commit()

    def get_execution(self, id_: str) -> sqlite3.Row | None:
        return self.db.execute("SELECT * FROM executions WHERE id = ?", (id_,)).fetchone()

    def take_lease(self, id_: str, owner_token: str, lease_until: int, now: int) -> None:
        cur = self.db.execute(
            """UPDATE executions
               SET owner_token = ?, lease_until = ?
               WHERE id = ? AND (owner_token IS NULL OR owner_token = ? OR lease_until IS NULL OR lease_until < ?)""",
            (owner_token, lease_until, id_, owner_token, now),
        )
        self.db.commit()
        if cur.rowcount != 1:
            raise RuntimeError(f"execution `{id_}` is owned by another worker")

    def get_continuation(self, execution_id: str) -> sqlite3.Row | None:
        return self.db.execute(
            "SELECT revision, json FROM continuations WHERE execution_id = ?", (execution_id,)
        ).fetchone()

    def commit_checkpoint(
        self, execution_id: str, revision: int, json: str, status: str, owner_token: str
    ) -> None:
        self.db.execute("BEGIN IMMEDIATE")
        try:
            current = self.get_execution(execution_id)
            if current is None:
                raise RuntimeError(f"unknown execution `{execution_id}`")
            if current["owner_token"] != owner_token:
                raise RuntimeError(f"execution `{execution_id}` is owned by another worker")
            if current["revision"] != revision - 1:
                raise RuntimeError(
                    f"revision conflict for `{execution_id}`: expected {revision - 1}, found {current['revision']}"
                )
            existing = self.get_continuation(execution_id)
            if existing is None:
                self.db.execute(
                    "INSERT INTO continuations(execution_id, revision, json) VALUES (?, ?, ?)",
                    (execution_id, revision, json),
                )
            else:
                updated = self.db.execute(
                    "UPDATE continuations SET revision = ?, json = ? WHERE execution_id = ? AND revision = ?",
                    (revision, json, execution_id, current["revision"]),
                )
                if updated.rowcount != 1:
                    raise RuntimeError(f"continuation cas failed for `{execution_id}`")
            exec_updated = self.db.execute(
                "UPDATE executions SET revision = ?, status = ? WHERE id = ? AND revision = ? AND owner_token = ?",
                (revision, status, execution_id, current["revision"], owner_token),
            )
            if exec_updated.rowcount != 1:
                raise RuntimeError(f"execution cas failed for `{execution_id}`")
            self.db.execute("COMMIT")
        except Exception:
            self.db.execute("ROLLBACK")
            raise

    def get_effect(self, execution_id: str, key: str) -> sqlite3.Row | None:
        return self.db.execute(
            "SELECT * FROM effects WHERE execution_id = ? AND key = ?", (execution_id, key)
        ).fetchone()

    def mark_effect_started(self, execution_id: str, key: str, idempotency_key: str) -> None:
        self.db.execute(
            """INSERT INTO effects(execution_id, key, idempotency_key, status, result_json)
               VALUES (?, ?, ?, 'started', NULL)
               ON CONFLICT(execution_id, key) DO UPDATE SET
                 idempotency_key = excluded.idempotency_key
               WHERE effects.status != 'completed'""",
            (execution_id, key, idempotency_key),
        )
        self.db.commit()

    def complete_effect(self, execution_id: str, key: str, idempotency_key: str, result_json: str) -> None:
        self.db.execute(
            """INSERT INTO effects(execution_id, key, idempotency_key, status, result_json)
               VALUES (?, ?, ?, 'completed', ?)
               ON CONFLICT(execution_id, key) DO UPDATE SET
                 status = 'completed',
                 idempotency_key = excluded.idempotency_key,
                 result_json = excluded.result_json""",
            (execution_id, key, idempotency_key, result_json),
        )
        self.db.commit()

    def fail_effect(self, execution_id: str, key: str, idempotency_key: str, result_json: str) -> None:
        self.db.execute(
            """INSERT INTO effects(execution_id, key, idempotency_key, status, result_json)
               VALUES (?, ?, ?, 'failed', ?)
               ON CONFLICT(execution_id, key) DO UPDATE SET
                 status = 'failed',
                 idempotency_key = excluded.idempotency_key,
                 result_json = excluded.result_json
               WHERE effects.status != 'completed'""",
            (execution_id, key, idempotency_key, result_json),
        )
        self.db.commit()

    def upsert_wait(self, wait_id: str, execution_id: str, event_name: str) -> None:
        self.db.execute(
            """INSERT INTO waits(wait_id, execution_id, event_name, status)
               VALUES (?, ?, ?, 'pending')
               ON CONFLICT(wait_id) DO NOTHING""",
            (wait_id, execution_id, event_name),
        )
        self.db.commit()

    def pending_wait(self, execution_id: str) -> sqlite3.Row | None:
        return self.db.execute(
            "SELECT * FROM waits WHERE execution_id = ? AND status = 'pending' LIMIT 1",
            (execution_id,),
        ).fetchone()

    def resolve_wait(self, wait_id: str) -> None:
        self.db.execute("UPDATE waits SET status = 'resolved' WHERE wait_id = ?", (wait_id,))
        self.db.commit()

    def enqueue_event(self, execution_id: str, name: str, payload_json: str) -> None:
        self.db.execute(
            "INSERT INTO events(execution_id, name, payload_json, consumed) VALUES (?, ?, ?, 0)",
            (execution_id, name, payload_json),
        )
        self.db.commit()

    def take_event(self, execution_id: str, name: str) -> sqlite3.Row | None:
        row = self.db.execute(
            "SELECT id, payload_json FROM events WHERE execution_id = ? AND name = ? AND consumed = 0 ORDER BY id LIMIT 1",
            (execution_id, name),
        ).fetchone()
        if row is None:
            return None
        self.db.execute("UPDATE events SET consumed = 1 WHERE id = ?", (row["id"],))
        self.db.commit()
        return row

    def upsert_timer(self, execution_id: str, resume_at_ms: int) -> None:
        self.db.execute(
            """INSERT INTO timers(execution_id, resume_at_ms, status)
               VALUES (?, ?, 'pending')
               ON CONFLICT(execution_id) DO NOTHING""",
            (execution_id, resume_at_ms),
        )
        self.db.commit()

    def pending_timer(self, execution_id: str) -> sqlite3.Row | None:
        return self.db.execute(
            "SELECT resume_at_ms, status FROM timers WHERE execution_id = ?", (execution_id,)
        ).fetchone()

    def resolve_timer(self, execution_id: str) -> None:
        self.db.execute("UPDATE timers SET status = 'resolved' WHERE execution_id = ?", (execution_id,))
        self.db.commit()

    def upsert_child(
        self, invoke_id: str, parent_execution_id: str, child_execution_id: str, flow_name: str
    ) -> None:
        self.db.execute(
            """INSERT INTO children(invoke_id, parent_execution_id, child_execution_id, flow_name, status, result_json)
               VALUES (?, ?, ?, ?, 'pending', NULL)
               ON CONFLICT(invoke_id) DO NOTHING""",
            (invoke_id, parent_execution_id, child_execution_id, flow_name),
        )
        self.db.commit()

    def get_child(self, invoke_id: str) -> sqlite3.Row | None:
        return self.db.execute("SELECT * FROM children WHERE invoke_id = ?", (invoke_id,)).fetchone()

    def complete_child(self, invoke_id: str, result_json: str) -> None:
        self.db.execute(
            "UPDATE children SET status = 'completed', result_json = ? WHERE invoke_id = ?",
            (result_json, invoke_id),
        )
        self.db.commit()
