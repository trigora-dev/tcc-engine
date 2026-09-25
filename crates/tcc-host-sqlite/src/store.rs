use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Transaction};

#[derive(Debug)]
pub struct StoreError {
    pub message: String,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self {
            message: err.to_string(),
        }
    }
}

fn err(message: impl Into<String>) -> StoreError {
    StoreError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRow {
    pub id: String,
    pub artifact_hash: String,
    pub revision: u64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRow {
    pub revision: u64,
    pub body: String,
}

struct EffectWrite<'a> {
    execution_id: &'a str,
    key: &'a str,
    idempotency_key: &'a str,
    input: &'a str,
    status: &'a str,
    result: Option<&'a str>,
    commit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRow {
    pub key: String,
    pub idempotency_key: String,
    pub status: String,
    pub input: String,
    pub result: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitRow {
    pub id: String,
    pub execution_id: String,
    pub event_name: String,
    pub status: String,
    pub payload: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerRow {
    pub execution_id: String,
    pub branch: String,
    pub wake_at_ms: i64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildRow {
    pub invoke_id: String,
    pub parent_execution_id: String,
    pub child_execution_id: String,
    pub program_name: String,
    pub artifact_hash: String,
    pub args_json: Option<String>,
    pub status: String,
    pub result_json: Option<String>,
}

/// Full continuation body plus rows that commit in the same transaction.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub execution_id: String,
    pub revision: u64,
    pub status: String,
    pub body: String,
    pub child: Option<NewChild>,
    pub completed_child: Option<CompletedChild>,
}

#[derive(Debug, Clone)]
pub struct NewChild {
    pub invoke_id: String,
    pub parent_execution_id: String,
    pub child_execution_id: String,
    pub program_name: String,
    pub artifact_hash: String,
    pub artifact_body: String,
    pub args_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CompletedChild {
    pub child_execution_id: String,
    pub result_json: String,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|io| err(io.to_string()))?;
            }
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS artifact (
                hash TEXT PRIMARY KEY,
                body TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS execution (
                id TEXT PRIMARY KEY,
                artifact_hash TEXT NOT NULL,
                revision INTEGER NOT NULL,
                status TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS checkpoint (
                execution_id TEXT PRIMARY KEY,
                revision INTEGER NOT NULL,
                body TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS effect (
                execution_id TEXT NOT NULL,
                key TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                status TEXT NOT NULL,
                input TEXT NOT NULL,
                result TEXT,
                PRIMARY KEY (execution_id, key)
            );
            CREATE TABLE IF NOT EXISTS wait (
                id TEXT PRIMARY KEY,
                execution_id TEXT NOT NULL,
                event_name TEXT NOT NULL,
                status TEXT NOT NULL,
                payload TEXT
            );
            CREATE TABLE IF NOT EXISTS event (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                execution_id TEXT NOT NULL,
                name TEXT NOT NULL,
                payload TEXT NOT NULL,
                consumed INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS timer (
                execution_id TEXT NOT NULL,
                branch TEXT NOT NULL,
                wake_at_ms INTEGER NOT NULL,
                status TEXT NOT NULL,
                PRIMARY KEY (execution_id, branch)
            );
            CREATE TABLE IF NOT EXISTS child (
                invoke_id TEXT PRIMARY KEY,
                parent_execution_id TEXT NOT NULL,
                child_execution_id TEXT NOT NULL,
                program_name TEXT NOT NULL,
                artifact_hash TEXT NOT NULL,
                args_json TEXT,
                status TEXT NOT NULL,
                result_json TEXT
            );
            ",
        )?;
        Ok(())
    }

    fn write<T>(
        &mut self,
        commit: bool,
        write: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let tx = self.conn.transaction()?;
        let value = write(&tx)?;
        if commit {
            tx.commit()?;
        }
        Ok(value)
    }

    pub fn put_artifact(&mut self, hash: &str, body: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO artifact(hash, body) VALUES (?1, ?2)
             ON CONFLICT(hash) DO UPDATE SET body = excluded.body",
            params![hash, body],
        )?;
        Ok(())
    }

    pub fn artifact(&self, hash: &str) -> Result<Option<String>, StoreError> {
        let body = self
            .conn
            .query_row(
                "SELECT body FROM artifact WHERE hash = ?1",
                params![hash],
                |row| row.get(0),
            )
            .optional()?;
        Ok(body)
    }

    pub fn create_execution(&mut self, id: &str, artifact_hash: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO execution(id, artifact_hash, revision, status) VALUES (?1, ?2, 0, 'running')",
            params![id, artifact_hash],
        )?;
        Ok(())
    }

    pub fn execution(&self, id: &str) -> Result<Option<ExecutionRow>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT id, artifact_hash, revision, status FROM execution WHERE id = ?1",
                params![id],
                |row| {
                    Ok(ExecutionRow {
                        id: row.get(0)?,
                        artifact_hash: row.get(1)?,
                        revision: row.get::<_, i64>(2)? as u64,
                        status: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn commit_snapshot(&mut self, snap: &Snapshot) -> Result<(), StoreError> {
        self.write_snapshot(snap, true)
    }

    /// Insert the snapshot, then roll the transaction back. The reopen must not see it.
    pub fn rollback_snapshot(&mut self, snap: &Snapshot) -> Result<(), StoreError> {
        self.write_snapshot(snap, false)
    }

    fn write_snapshot(&mut self, snap: &Snapshot, commit: bool) -> Result<(), StoreError> {
        let snap = snap.clone();
        self.write(commit, |tx| {
            if let Some(child) = &snap.child {
                tx.execute(
                    "INSERT INTO artifact(hash, body) VALUES (?1, ?2)
                     ON CONFLICT(hash) DO UPDATE SET body = excluded.body",
                    params![child.artifact_hash, child.artifact_body],
                )?;
                tx.execute(
                    "INSERT INTO child(
                        invoke_id, parent_execution_id, child_execution_id, program_name,
                        artifact_hash, args_json, status, result_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ready', NULL)",
                    params![
                        child.invoke_id,
                        child.parent_execution_id,
                        child.child_execution_id,
                        child.program_name,
                        child.artifact_hash,
                        child.args_json,
                    ],
                )?;
            }
            if let Some(done) = &snap.completed_child {
                tx.execute(
                    "UPDATE child SET status = 'completed', result_json = ?1
                     WHERE child_execution_id = ?2",
                    params![done.result_json, done.child_execution_id],
                )?;
            }
            tx.execute(
                "INSERT INTO checkpoint(execution_id, revision, body) VALUES (?1, ?2, ?3)
                 ON CONFLICT(execution_id) DO UPDATE SET revision = excluded.revision, body = excluded.body",
                params![snap.execution_id, snap.revision as i64, snap.body],
            )?;
            tx.execute(
                "UPDATE execution SET revision = ?1, status = ?2 WHERE id = ?3",
                params![snap.revision as i64, snap.status, snap.execution_id],
            )?;
            Ok(())
        })
    }

    pub fn checkpoint(&self, execution_id: &str) -> Result<Option<CheckpointRow>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT revision, body FROM checkpoint WHERE execution_id = ?1",
                params![execution_id],
                |row| {
                    Ok(CheckpointRow {
                        revision: row.get::<_, i64>(0)? as u64,
                        body: row.get(1)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn effect(&self, execution_id: &str, key: &str) -> Result<Option<EffectRow>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT key, idempotency_key, status, input, result
                 FROM effect WHERE execution_id = ?1 AND key = ?2",
                params![execution_id, key],
                |row| {
                    Ok(EffectRow {
                        key: row.get(0)?,
                        idempotency_key: row.get(1)?,
                        status: row.get(2)?,
                        input: row.get(3)?,
                        result: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn mark_effect_started(
        &mut self,
        execution_id: &str,
        key: &str,
        idempotency_key: &str,
        input: &str,
    ) -> Result<(), StoreError> {
        if input.is_empty() {
            return Err(err("effect input_json is required"));
        }
        self.conn.execute(
            "INSERT INTO effect(execution_id, key, idempotency_key, status, input, result)
             VALUES (?1, ?2, ?3, 'started', ?4, NULL)
             ON CONFLICT(execution_id, key) DO UPDATE SET
                idempotency_key = excluded.idempotency_key,
                status = 'started',
                input = excluded.input,
                result = NULL",
            params![execution_id, key, idempotency_key, input],
        )?;
        Ok(())
    }

    pub fn complete_effect(
        &mut self,
        execution_id: &str,
        key: &str,
        idempotency_key: &str,
        input: &str,
        result: &str,
    ) -> Result<(), StoreError> {
        self.write_effect(EffectWrite {
            execution_id,
            key,
            idempotency_key,
            input,
            status: "completed",
            result: Some(result),
            commit: true,
        })
    }

    pub fn rollback_effect(
        &mut self,
        execution_id: &str,
        key: &str,
        idempotency_key: &str,
        input: &str,
        result: &str,
    ) -> Result<(), StoreError> {
        self.write_effect(EffectWrite {
            execution_id,
            key,
            idempotency_key,
            input,
            status: "completed",
            result: Some(result),
            commit: false,
        })
    }

    pub fn fail_effect(
        &mut self,
        execution_id: &str,
        key: &str,
        idempotency_key: &str,
        input: &str,
        result: &str,
    ) -> Result<(), StoreError> {
        self.write_effect(EffectWrite {
            execution_id,
            key,
            idempotency_key,
            input,
            status: "failed",
            result: Some(result),
            commit: true,
        })
    }

    fn write_effect(&mut self, effect: EffectWrite<'_>) -> Result<(), StoreError> {
        if effect.input.is_empty() {
            return Err(err("effect input_json is required"));
        }
        let execution_id = effect.execution_id.to_string();
        let key = effect.key.to_string();
        let idempotency_key = effect.idempotency_key.to_string();
        let input = effect.input.to_string();
        let status = effect.status.to_string();
        let result = effect.result.map(str::to_string);
        self.write(effect.commit, |tx| {
            tx.execute(
                "INSERT INTO effect(execution_id, key, idempotency_key, status, input, result)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(execution_id, key) DO UPDATE SET
                    idempotency_key = excluded.idempotency_key,
                    status = excluded.status,
                    input = excluded.input,
                    result = excluded.result",
                params![execution_id, key, idempotency_key, status, input, result],
            )?;
            Ok(())
        })
    }

    pub fn upsert_wait(
        &mut self,
        id: &str,
        execution_id: &str,
        event_name: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO wait(id, execution_id, event_name, status, payload)
             VALUES (?1, ?2, ?3, 'pending', NULL)
             ON CONFLICT(id) DO UPDATE SET
                execution_id = excluded.execution_id,
                event_name = excluded.event_name,
                status = 'pending',
                payload = NULL",
            params![id, execution_id, event_name],
        )?;
        Ok(())
    }

    pub fn wait(&self, id: &str) -> Result<Option<WaitRow>, StoreError> {
        self.wait_query(
            "SELECT id, execution_id, event_name, status, payload FROM wait WHERE id = ?1",
            params![id],
        )
    }

    pub fn pending_wait(&self, execution_id: &str) -> Result<Option<WaitRow>, StoreError> {
        self.wait_query(
            "SELECT id, execution_id, event_name, status, payload
             FROM wait WHERE execution_id = ?1 AND status = 'pending'
             ORDER BY id LIMIT 1",
            params![execution_id],
        )
    }

    fn wait_query(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Option<WaitRow>, StoreError> {
        let row = self
            .conn
            .query_row(sql, params, |row| {
                Ok(WaitRow {
                    id: row.get(0)?,
                    execution_id: row.get(1)?,
                    event_name: row.get(2)?,
                    status: row.get(3)?,
                    payload: row.get(4)?,
                })
            })
            .optional()?;
        Ok(row)
    }

    pub fn resolve_wait(&mut self, id: &str, payload: &str) -> Result<(), StoreError> {
        self.write_wait(id, payload, true)
    }

    pub fn rollback_wait(&mut self, id: &str, payload: &str) -> Result<(), StoreError> {
        self.write_wait(id, payload, false)
    }

    fn write_wait(&mut self, id: &str, payload: &str, commit: bool) -> Result<(), StoreError> {
        let id = id.to_string();
        let payload = payload.to_string();
        self.write(commit, |tx| {
            tx.execute(
                "UPDATE wait SET status = 'resolved', payload = ?1 WHERE id = ?2",
                params![payload, id],
            )?;
            Ok(())
        })
    }

    pub fn enqueue_event(
        &mut self,
        execution_id: &str,
        name: &str,
        payload: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO event(execution_id, name, payload, consumed) VALUES (?1, ?2, ?3, 0)",
            params![execution_id, name, payload],
        )?;
        Ok(())
    }

    pub fn take_event(
        &mut self,
        execution_id: &str,
        name: &str,
    ) -> Result<Option<String>, StoreError> {
        let found: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, payload FROM event
                 WHERE execution_id = ?1 AND name = ?2 AND consumed = 0
                 ORDER BY id LIMIT 1",
                params![execution_id, name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((id, payload)) = found else {
            return Ok(None);
        };
        self.conn
            .execute("UPDATE event SET consumed = 1 WHERE id = ?1", params![id])?;
        Ok(Some(payload))
    }

    pub fn upsert_timer(
        &mut self,
        execution_id: &str,
        branch: &str,
        wake_at_ms: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO timer(execution_id, branch, wake_at_ms, status)
             VALUES (?1, ?2, ?3, 'pending')
             ON CONFLICT(execution_id, branch) DO UPDATE SET
                wake_at_ms = excluded.wake_at_ms,
                status = 'pending'",
            params![execution_id, branch, wake_at_ms],
        )?;
        Ok(())
    }

    pub fn pending_timer(
        &self,
        execution_id: &str,
        branch: Option<&str>,
    ) -> Result<Option<TimerRow>, StoreError> {
        let row = match branch {
            Some(branch) => self
                .conn
                .query_row(
                    "SELECT execution_id, branch, wake_at_ms, status FROM timer
                     WHERE execution_id = ?1 AND branch = ?2 AND status = 'pending'",
                    params![execution_id, branch],
                    read_timer,
                )
                .optional()?,
            None => self
                .conn
                .query_row(
                    "SELECT execution_id, branch, wake_at_ms, status FROM timer
                     WHERE execution_id = ?1 AND status = 'pending'
                     ORDER BY branch LIMIT 1",
                    params![execution_id],
                    read_timer,
                )
                .optional()?,
        };
        Ok(row)
    }

    pub fn timer(&self, execution_id: &str, branch: &str) -> Result<Option<TimerRow>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT execution_id, branch, wake_at_ms, status FROM timer
                 WHERE execution_id = ?1 AND branch = ?2",
                params![execution_id, branch],
                read_timer,
            )
            .optional()?;
        Ok(row)
    }

    pub fn due_timers(&self, now_ms: i64) -> Result<Vec<TimerRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT execution_id, branch, wake_at_ms, status FROM timer
             WHERE status = 'pending' AND wake_at_ms <= ?1
             ORDER BY wake_at_ms, execution_id, branch",
        )?;
        let rows = stmt.query_map(params![now_ms], read_timer)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn resolve_timer(&mut self, execution_id: &str, branch: &str) -> Result<(), StoreError> {
        self.write_timer(execution_id, branch, true)
    }

    pub fn rollback_timer(&mut self, execution_id: &str, branch: &str) -> Result<(), StoreError> {
        self.write_timer(execution_id, branch, false)
    }

    fn write_timer(
        &mut self,
        execution_id: &str,
        branch: &str,
        commit: bool,
    ) -> Result<(), StoreError> {
        let execution_id = execution_id.to_string();
        let branch = branch.to_string();
        self.write(commit, |tx| {
            tx.execute(
                "UPDATE timer SET status = 'resolved'
                 WHERE execution_id = ?1 AND branch = ?2",
                params![execution_id, branch],
            )?;
            Ok(())
        })
    }

    pub fn child(&self, invoke_id: &str) -> Result<Option<ChildRow>, StoreError> {
        self.child_query(
            "SELECT invoke_id, parent_execution_id, child_execution_id, program_name,
                    artifact_hash, args_json, status, result_json
             FROM child WHERE invoke_id = ?1",
            params![invoke_id],
        )
    }

    pub fn child_by_execution(
        &self,
        child_execution_id: &str,
    ) -> Result<Option<ChildRow>, StoreError> {
        self.child_query(
            "SELECT invoke_id, parent_execution_id, child_execution_id, program_name,
                    artifact_hash, args_json, status, result_json
             FROM child WHERE child_execution_id = ?1",
            params![child_execution_id],
        )
    }

    fn child_query(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Option<ChildRow>, StoreError> {
        let row = self
            .conn
            .query_row(sql, params, |row| {
                Ok(ChildRow {
                    invoke_id: row.get(0)?,
                    parent_execution_id: row.get(1)?,
                    child_execution_id: row.get(2)?,
                    program_name: row.get(3)?,
                    artifact_hash: row.get(4)?,
                    args_json: row.get(5)?,
                    status: row.get(6)?,
                    result_json: row.get(7)?,
                })
            })
            .optional()?;
        Ok(row)
    }

    pub fn ready_children(&self, parent_execution_id: &str) -> Result<Vec<ChildRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT invoke_id, parent_execution_id, child_execution_id, program_name,
                    artifact_hash, args_json, status, result_json
             FROM child WHERE parent_execution_id = ?1 AND status = 'ready'
             ORDER BY invoke_id",
        )?;
        let rows = stmt.query_map(params![parent_execution_id], |row| {
            Ok(ChildRow {
                invoke_id: row.get(0)?,
                parent_execution_id: row.get(1)?,
                child_execution_id: row.get(2)?,
                program_name: row.get(3)?,
                artifact_hash: row.get(4)?,
                args_json: row.get(5)?,
                status: row.get(6)?,
                result_json: row.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

fn read_timer(row: &rusqlite::Row<'_>) -> rusqlite::Result<TimerRow> {
    Ok(TimerRow {
        execution_id: row.get(0)?,
        branch: row.get(1)?,
        wake_at_ms: row.get(2)?,
        status: row.get(3)?,
    })
}
