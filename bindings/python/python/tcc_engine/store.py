# Copyright (c) 2026 Trigora, Inc.
# SPDX-License-Identifier: BUSL-1.1
# See LICENSE for full terms.

"""SQLite persistence for the Python reference host."""

from __future__ import annotations

import json
import sqlite3
from collections.abc import Callable
from time import perf_counter
from typing import Any

from tcc_engine.persist import (
    MATERIALIZE_EVERY,
    PYTHON_ADAPTIVE_THRESHOLDS,
    PYTHON_PACKING_DEFAULT,
    choose_packed_kind,
    packing_from_env,
    packing_thresholds_from_env,
    persist_mode_from_env,
    reconstruct_continuation,
)

HostObserver = Callable[[dict[str, Any]], None]


def _engine_version() -> str:
    try:
        from tcc_engine.compile import PACKAGE_VERSION

        return PACKAGE_VERSION
    except Exception:
        return "26.10.1"


def emit_host_event(observer: HostObserver | None, event: dict[str, Any]) -> None:
    if observer is None:
        return
    try:
        observer({**event, "engineVersion": _engine_version()})
    except Exception:
        return


class Store:
    def __init__(
        self,
        path: str,
        persist: str | None = None,
        on_event: HostObserver | None = None,
        packing: str | None = None,
        min_full_bytes: float | None = None,
        max_delta_ratio: float | None = None,
    ) -> None:
        self.persist = persist or persist_mode_from_env()
        self._observer = on_event
        self.packing = packing or packing_from_env(PYTHON_PACKING_DEFAULT)
        env_thresholds = packing_thresholds_from_env(PYTHON_ADAPTIVE_THRESHOLDS)
        self.min_full_bytes = float(env_thresholds["min_full_bytes"] if min_full_bytes is None else min_full_bytes)
        self.max_delta_ratio = float(env_thresholds["max_delta_ratio"] if max_delta_ratio is None else max_delta_ratio)
        self.db = sqlite3.connect(path)
        self.db.isolation_level = None
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode = WAL")
        self.db.execute("PRAGMA foreign_keys = ON")
        self.db.execute("PRAGMA busy_timeout = 5000")
        self._group_held = 0
        self._pending: list[dict[str, Any]] = []
        self._created_children: list[dict[str, Any]] = []
        self._metric_bytes_written = 0
        self._metric_snapshot_count = 0
        self._metric_delta_count = 0
        self._metric_commit_count = 0
        self._metric_delta_bytes_written = 0
        self._metric_batch_occupancy_samples: list[int] = []
        self._queued_events: list[dict[str, Any]] = []
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
              input_json TEXT NOT NULL,
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
            """
        )
        self._ensure_effect_input_column()

    def _ensure_effect_input_column(self) -> None:
        columns = list(self.db.execute("PRAGMA table_info(effects)"))
        input_column = next((row for row in columns if row[1] == "input_json"), None)
        if input_column is not None and input_column[3] == 1:
            return
        self.db.execute("DROP TABLE effects")
        self.db.execute(
            """CREATE TABLE effects (
              execution_id TEXT NOT NULL,
              key TEXT NOT NULL,
              idempotency_key TEXT NOT NULL,
              status TEXT NOT NULL,
              result_json TEXT,
              input_json TEXT NOT NULL,
              PRIMARY KEY (execution_id, key)
            )"""
        )
        self.db.commit()

    def close(self) -> None:
        self.db.close()

    def observe(self, event: dict[str, Any]) -> None:
        emit_host_event(self._observer, event)

    def reset_metrics(self) -> None:
        self._metric_bytes_written = 0
        self._metric_snapshot_count = 0
        self._metric_delta_count = 0
        self._metric_commit_count = 0
        self._metric_delta_bytes_written = 0
        self._metric_batch_occupancy_samples = []

    def metrics(self) -> dict[str, Any]:
        return {
            "bytesWritten": self._metric_bytes_written,
            "snapshotCount": self._metric_snapshot_count,
            "deltaCount": self._metric_delta_count,
            "commitCount": self._metric_commit_count,
            "batchOccupancySamples": list(self._metric_batch_occupancy_samples),
            "deltaBytesWritten": self._metric_delta_bytes_written,
        }

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

    def get_continuation(
        self, execution_id: str, observe_restore: bool = False
    ) -> sqlite3.Row | dict[str, Any] | None:
        if self.persist == "replay":
            return None
        if self.persist == "optimized":
            reconstructed = self._reconstruct(execution_id, observe_restore)
            if reconstructed is not None:
                return reconstructed
        started = perf_counter() if observe_restore else 0.0
        saved = self.db.execute(
            "SELECT revision, json FROM continuations WHERE execution_id = ?", (execution_id,)
        ).fetchone()
        if saved is not None and observe_restore:
            self.observe(
                {
                    "type": "continuation.restored",
                    "executionId": execution_id,
                    "revision": saved["revision"],
                    "durationMs": round((perf_counter() - started) * 1000, 4),
                }
            )
        return saved

    def begin_group(self) -> None:
        self._group_held += 1

    def is_grouping(self) -> bool:
        return self._group_held > 0

    def abort_group(self) -> None:
        self._group_held = 0
        self._pending = []
        self._created_children = []

    def take_created_children(self) -> list[dict[str, Any]]:
        created = self._created_children
        self._created_children = []
        return created

    def end_group(self) -> None:
        if self._group_held == 0:
            return
        self._group_held -= 1
        if self._group_held == 0:
            self.flush_group()

    def flush_group(self) -> None:
        if not self._pending:
            return
        batch = self._pending
        self._pending = []
        self._queued_events = []
        before = (self._metric_bytes_written, self._metric_snapshot_count, self._metric_delta_count, self._metric_delta_bytes_written)
        started = perf_counter()
        self.db.execute("BEGIN IMMEDIATE")
        try:
            created: list[dict[str, Any]] = []
            for op in batch:
                kind = op.get("type") or "checkpoint"
                if kind == "createChild":
                    self._apply_create_child(op)
                    created.append(op)
                elif kind == "completeChild":
                    self._apply_complete_child(op)
                else:
                    self._apply_checkpoint(op)
            self.db.execute("COMMIT")
            self._metric_commit_count += 1
            self._metric_batch_occupancy_samples.append(sum(1 for op in batch if (op.get("type") or "checkpoint") == "checkpoint"))
            self._created_children.extend(created)
            queued = self._queued_events
            self._queued_events = []
            for event in queued:
                self.observe(event)
            self.observe(
                {
                    "type": "batch.committed",
                    "batchSize": len(batch),
                    "durationMs": round((perf_counter() - started) * 1000, 4),
                }
            )
        except Exception as err:
            self.db.execute("ROLLBACK")
            self._queued_events = []
            (self._metric_bytes_written, self._metric_snapshot_count, self._metric_delta_count, self._metric_delta_bytes_written) = before
            self.observe({"type": "runtime.error", "message": str(err)})
            raise

    def enqueue_checkpoint(
        self,
        execution_id: str,
        revision: int,
        json_text: str,
        status: str,
        owner_token: str,
        *,
        kind: str = "snapshot",
        delta_json: str | None = None,
        materialize: bool = True,
    ) -> None:
        self._pending.append(
            {
                "type": "checkpoint",
                "execution_id": execution_id,
                "revision": revision,
                "json": json_text,
                "status": status,
                "owner_token": owner_token,
                "kind": kind,
                "delta_json": delta_json,
                "materialize": materialize,
            }
        )

    def enqueue_create_child(self, op: dict[str, Any]) -> None:
        self._pending.append({"type": "createChild", **op})

    def enqueue_complete_child(self, invoke_id: str, result_json: str) -> None:
        self._pending.append({"type": "completeChild", "invoke_id": invoke_id, "result_json": result_json})

    def flush_if_ungrouped(self) -> None:
        if self._group_held == 0:
            self.flush_group()

    def commit_checkpoint(
        self,
        execution_id: str,
        revision: int,
        json_text: str,
        status: str,
        owner_token: str,
        *,
        kind: str = "snapshot",
        delta_json: str | None = None,
        materialize: bool = True,
    ) -> None:
        self.enqueue_checkpoint(
            execution_id,
            revision,
            json_text,
            status,
            owner_token,
            kind=kind,
            delta_json=delta_json,
            materialize=materialize,
        )
        self.flush_if_ungrouped()

    def exec_head(self, execution_id: str) -> sqlite3.Row | None:
        return self.db.execute("SELECT * FROM exec_head WHERE execution_id = ?", (execution_id,)).fetchone()

    def wal_records(self, execution_id: str) -> list[sqlite3.Row]:
        return self.db.execute(
            "SELECT seq, revision, kind, payload FROM persist_wal WHERE execution_id = ? ORDER BY revision",
            (execution_id,),
        ).fetchall()

    def wal_row_count(self) -> int:
        row = self.db.execute("SELECT COUNT(*) AS n FROM persist_wal").fetchone()
        return int(row["n"])

    def recover_plan(self, execution_id: str) -> dict[str, Any] | None:
        head = self.exec_head(execution_id)
        if head is None:
            return None
        plan = self.db.execute(
            """EXPLAIN QUERY PLAN
               SELECT payload FROM persist_wal
               WHERE execution_id = ? AND revision > ? AND revision <= ?
               ORDER BY revision""",
            (execution_id, head["snapshot_revision"], head["revision"]),
        ).fetchall()
        detail = "\n".join(str(row["detail"]) for row in plan)
        return {
            "snapshot_revision": head["snapshot_revision"],
            "suffix_length": head["revision"] - head["snapshot_revision"],
            "used_index": "index" in detail.lower(),
            "detail": detail,
        }

    def retained_wal_bytes(self, execution_id: str | None = None) -> int:
        if execution_id is None:
            row = self.db.execute("SELECT COALESCE(SUM(LENGTH(payload)), 0) AS n FROM persist_wal").fetchone()
        else:
            row = self.db.execute(
                "SELECT COALESCE(SUM(LENGTH(payload)), 0) AS n FROM persist_wal WHERE execution_id = ?",
                (execution_id,),
            ).fetchone()
        return int(row["n"])

    def _reconstruct(self, execution_id: str, observe_restore: bool = False) -> dict[str, Any] | None:
        head = self.exec_head(execution_id)
        if head is None:
            return None
        started = perf_counter() if observe_restore else 0.0
        snapshot = self.db.execute(
            "SELECT payload FROM persist_wal WHERE execution_id = ? AND revision = ? AND kind = 'snapshot'",
            (execution_id, head["snapshot_revision"]),
        ).fetchone()
        if snapshot is None:
            raise RuntimeError(
                f"missing snapshot revision {head['snapshot_revision']} for `{execution_id}`"
            )
        suffix = self.db.execute(
            """SELECT payload FROM persist_wal
               WHERE execution_id = ? AND revision > ? AND revision <= ?
               ORDER BY revision""",
            (execution_id, head["snapshot_revision"], head["revision"]),
        ).fetchall()
        if len(suffix) > MATERIALIZE_EVERY:
            raise RuntimeError(
                f"WAL suffix for `{execution_id}` exceeds materialization bound {MATERIALIZE_EVERY}"
            )
        reconstructed = reconstruct_continuation(
            json.loads(snapshot["payload"]),
            [json.loads(row["payload"]) for row in suffix],
            head["revision"],
        )
        if observe_restore:
            self.observe(
                {
                    "type": "continuation.restored",
                    "executionId": execution_id,
                    "revision": head["revision"],
                    "durationMs": round((perf_counter() - started) * 1000, 4),
                    "suffixLength": len(suffix),
                }
            )
        return {"revision": head["revision"], "json": json.dumps(reconstructed, separators=(",", ":"))}

    def _apply_checkpoint(self, write: dict[str, Any]) -> None:
        current = self.get_execution(write["execution_id"])
        if current is None:
            raise RuntimeError(f"unknown execution `{write['execution_id']}`")
        if current["owner_token"] != write["owner_token"]:
            raise RuntimeError(f"execution `{write['execution_id']}` is owned by another worker")
        if self.persist == "replay" and current["revision"] >= write["revision"]:
            return
        if current["revision"] != write["revision"] - 1:
            raise RuntimeError(
                f"revision conflict for `{write['execution_id']}`: expected {write['revision'] - 1}, found {current['revision']}"
            )
        if self.persist == "naive":
            self._apply_naive_checkpoint(write, current["revision"])
        elif self.persist == "replay":
            self._apply_replay_checkpoint(write)
        else:
            self._apply_optimized_checkpoint(write)
        exec_updated = self.db.execute(
            "UPDATE executions SET revision = ?, status = ? WHERE id = ? AND revision = ? AND owner_token = ?",
            (
                write["revision"],
                write["status"],
                write["execution_id"],
                current["revision"],
                write["owner_token"],
            ),
        )
        if exec_updated.rowcount != 1:
            raise RuntimeError(f"execution cas failed for `{write['execution_id']}`")

    def _apply_naive_checkpoint(self, write: dict[str, Any], current_revision: int) -> None:
        bytes_ = len(write["json"].encode("utf-8"))
        self._metric_bytes_written += bytes_
        self._metric_snapshot_count += 1
        self._queued_events.extend(
            [
                {
                    "type": "checkpoint.persisted",
                    "executionId": write["execution_id"],
                    "revision": write["revision"],
                    "kind": "snapshot",
                    "bytes": bytes_,
                },
                {
                    "type": "checkpoint.materialized",
                    "executionId": write["execution_id"],
                    "revision": write["revision"],
                    "kind": "snapshot",
                    "bytes": bytes_,
                },
            ]
        )
        existing = self.db.execute(
            "SELECT revision FROM continuations WHERE execution_id = ?", (write["execution_id"],)
        ).fetchone()
        if existing is None:
            self.db.execute(
                "INSERT INTO continuations(execution_id, revision, json) VALUES (?, ?, ?)",
                (write["execution_id"], write["revision"], write["json"]),
            )
            return
        updated = self.db.execute(
            "UPDATE continuations SET revision = ?, json = ? WHERE execution_id = ? AND revision = ?",
            (write["revision"], write["json"], write["execution_id"], current_revision),
        )
        if updated.rowcount != 1:
            raise RuntimeError(f"continuation cas failed for `{write['execution_id']}`")

    def _apply_replay_checkpoint(self, write: dict[str, Any]) -> None:
        payload = json.dumps({"revision": write["revision"], "status": write["status"]}, separators=(",", ":"))
        bytes_ = len(payload.encode("utf-8"))
        self._metric_bytes_written += bytes_
        self._queued_events.append(
            {
                "type": "checkpoint.persisted",
                "executionId": write["execution_id"],
                "revision": write["revision"],
                "bytes": bytes_,
            }
        )
        self.db.execute(
            "INSERT INTO persist_wal(execution_id, revision, kind, payload) VALUES (?, ?, 'history', ?)",
            (write["execution_id"], write["revision"], payload),
        )

    def _apply_optimized_checkpoint(self, write: dict[str, Any]) -> None:
        head = self.exec_head(write["execution_id"])
        requested_kind = write.get("kind") or "snapshot"
        force_snapshot = (
            write.get("materialize") is True
            or requested_kind == "snapshot"
            or (
                head is not None
                and write["revision"] - head["snapshot_revision"] > MATERIALIZE_EVERY
            )
        )
        full_bytes = len(write["json"].encode("utf-8"))
        delta_json = write.get("delta_json")
        delta_bytes = None if delta_json is None else len(delta_json.encode("utf-8"))
        kind = choose_packed_kind(
            must_materialize=force_snapshot,
            packing=self.packing,
            full_bytes=full_bytes,
            delta_bytes=delta_bytes,
            min_full_bytes=self.min_full_bytes,
            max_delta_ratio=self.max_delta_ratio,
        )
        payload = write["json"] if kind == "snapshot" else (write.get("delta_json") or write["json"])
        bytes_ = len(payload.encode("utf-8"))
        self._metric_bytes_written += bytes_
        if kind == "snapshot":
            self._metric_snapshot_count += 1
        else:
            self._metric_delta_count += 1
            self._metric_delta_bytes_written += bytes_
        self._queued_events.append(
            {
                "type": "checkpoint.persisted",
                "executionId": write["execution_id"],
                "revision": write["revision"],
                "kind": kind,
                "bytes": bytes_,
            }
        )
        if kind == "snapshot":
            self._queued_events.append(
                {
                    "type": "checkpoint.materialized",
                    "executionId": write["execution_id"],
                    "revision": write["revision"],
                    "kind": "snapshot",
                    "bytes": bytes_,
                }
            )
        cur = self.db.execute(
            "INSERT INTO persist_wal(execution_id, revision, kind, payload) VALUES (?, ?, ?, ?)",
            (write["execution_id"], write["revision"], kind, payload),
        )
        snapshot_revision = write["revision"] if kind == "snapshot" else None if head is None else head["snapshot_revision"]
        if snapshot_revision is None:
            raise RuntimeError(f"delta without snapshot for `{write['execution_id']}`")
        self.db.execute(
            """INSERT INTO exec_head(execution_id, revision, snapshot_revision, wal_seq, status)
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT(execution_id) DO UPDATE SET
                 revision = excluded.revision,
                 snapshot_revision = excluded.snapshot_revision,
                 wal_seq = excluded.wal_seq,
                 status = excluded.status""",
            (
                write["execution_id"],
                write["revision"],
                snapshot_revision,
                cur.lastrowid,
                write["status"],
            ),
        )

    def get_effect(self, execution_id: str, key: str) -> sqlite3.Row | None:
        return self.db.execute(
            "SELECT * FROM effects WHERE execution_id = ? AND key = ?", (execution_id, key)
        ).fetchone()

    def mark_effect_started(
        self, execution_id: str, key: str, idempotency_key: str, input_json: str
    ) -> None:
        self.db.execute(
            """INSERT INTO effects(execution_id, key, idempotency_key, status, result_json, input_json)
               VALUES (?, ?, ?, 'started', NULL, ?)
               ON CONFLICT(execution_id, key) DO UPDATE SET
                 idempotency_key = excluded.idempotency_key,
                 input_json = excluded.input_json
               WHERE effects.status != 'completed'""",
            (execution_id, key, idempotency_key, input_json),
        )
        self.db.commit()

    def complete_effect(
        self, execution_id: str, key: str, idempotency_key: str, result_json: str, input_json: str
    ) -> None:
        self.db.execute(
            """INSERT INTO effects(execution_id, key, idempotency_key, status, result_json, input_json)
               VALUES (?, ?, ?, 'completed', ?, ?)
               ON CONFLICT(execution_id, key) DO UPDATE SET
                 status = 'completed',
                 idempotency_key = excluded.idempotency_key,
                 result_json = excluded.result_json,
                 input_json = effects.input_json""",
            (execution_id, key, idempotency_key, result_json, input_json),
        )
        self.db.commit()

    def fail_effect(
        self, execution_id: str, key: str, idempotency_key: str, result_json: str, input_json: str
    ) -> None:
        self.db.execute(
            """INSERT INTO effects(execution_id, key, idempotency_key, status, result_json, input_json)
               VALUES (?, ?, ?, 'failed', ?, ?)
               ON CONFLICT(execution_id, key) DO UPDATE SET
                 status = 'failed',
                 idempotency_key = excluded.idempotency_key,
                 result_json = excluded.result_json,
                 input_json = effects.input_json
               WHERE effects.status != 'completed'""",
            (execution_id, key, idempotency_key, result_json, input_json),
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

    def get_wait(self, wait_id: str) -> sqlite3.Row | None:
        return self.db.execute("SELECT * FROM waits WHERE wait_id = ?", (wait_id,)).fetchone()

    def pending_wait(self, execution_id: str) -> sqlite3.Row | None:
        return self.db.execute(
            "SELECT * FROM waits WHERE execution_id = ? AND status = 'pending' ORDER BY rowid LIMIT 1",
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

    def upsert_timer(self, execution_id: str, resume_at_ms: int, branch_id: str = "") -> None:
        self.db.execute(
            """INSERT INTO timers(execution_id, branch_id, resume_at_ms, status)
               VALUES (?, ?, ?, 'pending')
               ON CONFLICT(execution_id, branch_id) DO NOTHING""",
            (execution_id, branch_id, resume_at_ms),
        )
        self.db.commit()

    def pending_timer(self, execution_id: str, branch_id: str = "") -> sqlite3.Row | None:
        return self.db.execute(
            """SELECT branch_id, resume_at_ms, status FROM timers
               WHERE execution_id = ? AND branch_id = ? AND status = 'pending'""",
            (execution_id, branch_id),
        ).fetchone()

    def resolve_timer(self, execution_id: str, branch_id: str = "") -> None:
        self.db.execute(
            "UPDATE timers SET status = 'resolved' WHERE execution_id = ? AND branch_id = ?",
            (execution_id, branch_id),
        )
        self.db.commit()

    def _apply_create_child(self, op: dict[str, Any]) -> None:
        args_json = _canonical_args_json(op.get("args_json"))
        existing = self.get_child(op["invoke_id"])
        if existing is not None:
            if existing["args_json"] != args_json:
                raise RuntimeError(f"child {op['invoke_id']} argument vector mismatch")
            return
        self.db.execute(
            """INSERT INTO children(invoke_id, parent_execution_id, child_execution_id, flow_name, status, result_json, args_json)
               VALUES (?, ?, ?, ?, 'pending', NULL, ?)
               ON CONFLICT(invoke_id) DO NOTHING""",
            (
                op["invoke_id"],
                op["parent_execution_id"],
                op["child_execution_id"],
                op["flow_name"],
                args_json,
            ),
        )
        stored = self.get_child(op["invoke_id"])
        if stored is None or stored["args_json"] != args_json:
            raise RuntimeError(f"child {op['invoke_id']} argument vector mismatch")
        self.db.execute(
            """INSERT OR IGNORE INTO executions(id, artifact_hash, revision, status, owner_token, lease_until)
               VALUES (?, ?, 0, 'runnable', ?, ?)""",
            (op["child_execution_id"], op["artifact_hash"], op["owner_token"], op["lease_until"]),
        )
        self._queued_events.append({"type": "child.created", "executionId": op["child_execution_id"]})

    def _apply_complete_child(self, op: dict[str, Any]) -> None:
        child = self.get_child(op["invoke_id"])
        self.db.execute(
            "UPDATE children SET status = 'completed', result_json = ? WHERE invoke_id = ?",
            (op["result_json"], op["invoke_id"]),
        )
        self._queued_events.append(
            {
                "type": "child.completed",
                "executionId": None if child is None else child["child_execution_id"],
            }
        )

    def upsert_child(
        self, invoke_id: str, parent_execution_id: str, child_execution_id: str, flow_name: str
    ) -> None:
        self.enqueue_create_child(
            {
                "invoke_id": invoke_id,
                "parent_execution_id": parent_execution_id,
                "child_execution_id": child_execution_id,
                "flow_name": flow_name,
                "artifact_hash": "",
                "owner_token": "",
                "lease_until": 0,
                "args_json": None,
            }
        )
        self.flush_if_ungrouped()

    def get_child(self, invoke_id: str) -> sqlite3.Row | None:
        return self.db.execute("SELECT * FROM children WHERE invoke_id = ?", (invoke_id,)).fetchone()

    def get_child_by_execution_id(self, child_execution_id: str) -> sqlite3.Row | None:
        return self.db.execute(
            "SELECT * FROM children WHERE child_execution_id = ?", (child_execution_id,)
        ).fetchone()

    def complete_child(self, invoke_id: str, result_json: str) -> None:
        self.enqueue_complete_child(invoke_id, result_json)
        self.flush_if_ungrouped()


def _canonical_args_json(text: str | None) -> str | None:
    if text is None or text == "[]":
        return None
    return text
