use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use tcc_core::{ChildSpec, EffectStatus, HostRequest, HostResponse};
use tcc_host::{Host, HostError};
use tcc_state::{
    decode_value, encode_continuation, encode_value, export_value, Continuation,
    ContinuationStatus, Value,
};

use crate::crash::Crash;
use crate::store::{CompletedChild, NewChild, Snapshot, Store};

pub struct SqliteHost {
    pub store: Store,
    pub execution_id: String,
    pub effects: HashMap<String, Value>,
    pub fail_counts: HashMap<String, u32>,
    pub effect_log: Option<PathBuf>,
    pub child_artifacts: HashMap<String, String>,
    pub crash: Crash,
    pub auto_deliver: bool,
    /// Set when the caller named a payload. Missing means auto-delivery uses `"ok"`.
    pub event_payload: Option<Value>,
    pub completion_reverse: bool,
    pub cancel: bool,
    continuation_json: Option<String>,
    staged_status: ContinuationStatus,
    staged_result: Option<Value>,
    staged_child: Option<NewChild>,
}

impl SqliteHost {
    pub fn open(path: &Path, execution_id: impl Into<String>) -> Result<Self, HostError> {
        Ok(Self {
            store: Store::open(path).map_err(store_err)?,
            execution_id: execution_id.into(),
            effects: HashMap::new(),
            fail_counts: HashMap::new(),
            effect_log: None,
            child_artifacts: HashMap::new(),
            crash: Crash::default(),
            auto_deliver: true,
            event_payload: None,
            completion_reverse: false,
            cancel: false,
            continuation_json: None,
            staged_status: ContinuationStatus::Runnable,
            staged_result: None,
            staged_child: None,
        })
    }

    pub fn stage_continuation(&mut self, continuation: &Continuation) -> Result<(), HostError> {
        let bytes = encode_continuation(continuation).map_err(state_err)?;
        self.continuation_json = Some(utf8(bytes)?);
        self.staged_status = continuation.status;
        self.staged_result = continuation
            .result
            .as_ref()
            .map(|value| export_value(&continuation.heap, value).unwrap_or_else(|_| value.clone()));
        Ok(())
    }

    fn crash_at(&mut self, hook: &str, detail: Option<&str>) -> Result<(), HostError> {
        self.crash
            .hit(hook, detail)
            .map_err(|hit| HostError::Message(hit.to_string()))
    }

    fn persist_checkpoint(&mut self, revision: u64) -> Result<HostResponse, HostError> {
        self.crash_at("before_persist_checkpoint", None)?;
        let raw = self.continuation_json.clone().ok_or_else(|| {
            HostError::Message("persist requires a staged continuation".to_string())
        })?;
        let body = patch_revision(&raw, revision)?;
        let status = status_name(self.staged_status);
        let completed_child = if self.staged_status == ContinuationStatus::Completed {
            match self
                .store
                .child_by_execution(&self.execution_id)
                .map_err(store_err)?
            {
                Some(_) => Some(CompletedChild {
                    child_execution_id: self.execution_id.clone(),
                    result_json: canonical(
                        self.staged_result.as_ref().unwrap_or(&Value::Undefined),
                    )?,
                }),
                None => None,
            }
        } else {
            None
        };
        let child = self.staged_child.take();
        let snap = Snapshot {
            execution_id: self.execution_id.clone(),
            revision,
            status: status.to_string(),
            body,
            child,
            completed_child,
        };
        if let Err(err) = self.store.commit_snapshot(&snap) {
            self.staged_child = snap.child;
            return Err(store_err(err));
        }
        self.crash_at("after_persist_checkpoint", None)?;
        if status == "suspended" {
            self.crash_at("after_wait_checkpoint", None)?;
        }
        Ok(HostResponse::PersistConfirmed { revision })
    }

    fn run_effect(
        &mut self,
        key: String,
        idempotency_key: String,
        input: Value,
    ) -> Result<HostResponse, HostError> {
        let input_json = canonical(&input)?;
        loop {
            self.crash_at("before_effect_provider", None)?;
            if let Some(existing) = self
                .store
                .effect(&self.execution_id, &key)
                .map_err(store_err)?
            {
                if existing.input != input_json {
                    return Err(HostError::Message(format!(
                        "effect {}:{} input mismatch",
                        self.execution_id, key
                    )));
                }
                if existing.status == "completed" {
                    let Some(result) = existing.result else {
                        return Err(HostError::Message(format!(
                            "effect {}:{} completed without a result",
                            self.execution_id, key
                        )));
                    };
                    let value = decode_value(result.as_bytes()).map_err(state_err)?;
                    return Ok(HostResponse::EffectResult { value });
                }
            }
            self.store
                .mark_effect_started(&self.execution_id, &key, &idempotency_key, &input_json)
                .map_err(store_err)?;
            if self.fail_counts.get(&key).copied().unwrap_or(0) > 0 {
                let left = self.fail_counts.get_mut(&key).expect("count exists");
                *left -= 1;
                self.append_log(&key)?;
                let failed = canonical(&Value::String("failed".to_string()))?;
                self.store
                    .fail_effect(
                        &self.execution_id,
                        &key,
                        &idempotency_key,
                        &input_json,
                        &failed,
                    )
                    .map_err(store_err)?;
                self.crash_at("after_persist_effect", Some(&key))?;
                continue;
            }
            self.append_log(&key)?;
            let produced = self
                .effects
                .get(&key)
                .cloned()
                .ok_or_else(|| HostError::Message(format!("no effect for `{key}`")))?;
            if matches!(&produced, Value::String(text) if text == "__fail__") {
                return Ok(HostResponse::EffectFailed {
                    message: "failed".to_string(),
                });
            }
            self.crash_at("after_effect_provider", None)?;
            return Ok(HostResponse::EffectResult { value: produced });
        }
    }

    fn persist_effect(
        &mut self,
        key: &str,
        idempotency_key: &str,
        status: EffectStatus,
        result: Option<&Value>,
    ) -> Result<HostResponse, HostError> {
        self.crash_at("before_persist_effect", None)?;
        let existing = self
            .store
            .effect(&self.execution_id, key)
            .map_err(store_err)?
            .ok_or_else(|| HostError::Message(format!("effect {key} was not started")))?;
        let stored = result
            .cloned()
            .unwrap_or(Value::String("failed".to_string()));
        let result_json = canonical(&stored)?;
        match status {
            EffectStatus::Failed => self
                .store
                .fail_effect(
                    &self.execution_id,
                    key,
                    idempotency_key,
                    &existing.input,
                    &result_json,
                )
                .map_err(store_err)?,
            EffectStatus::Completed | EffectStatus::Started => self
                .store
                .complete_effect(
                    &self.execution_id,
                    key,
                    idempotency_key,
                    &existing.input,
                    &result_json,
                )
                .map_err(store_err)?,
        }
        self.crash_at("after_persist_effect", Some(key))?;
        Ok(HostResponse::Ack)
    }

    fn append_log(&self, key: &str) -> Result<(), HostError> {
        let Some(path) = &self.effect_log else {
            return Ok(());
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|err| HostError::Message(err.to_string()))?;
        writeln!(file, "{key}").map_err(|err| HostError::Message(err.to_string()))?;
        file.sync_all()
            .map_err(|err| HostError::Message(err.to_string()))?;
        Ok(())
    }
}

impl Host for SqliteHost {
    fn handle(&mut self, request: HostRequest) -> Result<HostResponse, HostError> {
        match request {
            HostRequest::PersistCheckpoint { revision, .. } => self.persist_checkpoint(revision),
            HostRequest::RunEffect {
                key,
                idempotency_key,
                input,
            } => self.run_effect(key, idempotency_key, input),
            HostRequest::PersistEffect { record } => self.persist_effect(
                &record.key,
                &record.idempotency_key,
                record.status,
                record.result.as_ref(),
            ),
            HostRequest::RegisterTimer {
                resume_at_ms,
                branch,
            } => {
                let branch = branch.unwrap_or_default();
                let wake_at_ms = i64::try_from(resume_at_ms).map_err(|_| {
                    HostError::Message(format!("timer wakeAt {resume_at_ms} does not fit"))
                })?;
                self.store
                    .upsert_timer(&self.execution_id, &branch, wake_at_ms)
                    .map_err(store_err)?;
                self.crash_at("after_register_timer", None)?;
                Ok(HostResponse::Ack)
            }
            HostRequest::RegisterWait { wait } => {
                self.store
                    .upsert_wait(&wait.wait_id, &self.execution_id, &wait.event_name)
                    .map_err(store_err)?;
                self.crash_at("after_register_wait", None)?;
                Ok(HostResponse::Ack)
            }
            HostRequest::CreateChild { child } => {
                self.stage_child(child)?;
                self.crash_at("after_create_child", None)?;
                Ok(HostResponse::Ack)
            }
            HostRequest::FetchArtifact { hash } => {
                let found = self.store.artifact(&hash).map_err(store_err)?;
                if found.is_none() {
                    return Err(HostError::Message(format!("missing artifact `{hash}`")));
                }
                Ok(HostResponse::Artifact { hash })
            }
        }
    }
}

impl SqliteHost {
    fn stage_child(&mut self, child: ChildSpec) -> Result<(), HostError> {
        let artifact_body = self
            .child_artifacts
            .get(&child.program_name)
            .cloned()
            .ok_or_else(|| {
                HostError::Message(format!("no child artifact for `{}`", child.program_name))
            })?;
        let artifact_hash = artifact_hash(&artifact_body)?;
        let args_json = if child.args.is_empty() {
            None
        } else {
            Some(encode_args(&child.args)?)
        };
        self.staged_child = Some(NewChild {
            invoke_id: child.invoke_id,
            parent_execution_id: self.execution_id.clone(),
            child_execution_id: child.child_execution_id,
            program_name: child.program_name,
            artifact_hash,
            artifact_body,
            args_json,
        });
        Ok(())
    }
}

pub fn canonical(value: &Value) -> Result<String, HostError> {
    utf8(encode_value(value).map_err(state_err)?)
}

fn artifact_hash(body: &str) -> Result<String, HostError> {
    let json = tcc_state::json::Json::parse(body).map_err(state_err)?;
    let envelope = tcc_state::json::Json::get(json.as_object().map_err(state_err)?, "envelope")
        .map_err(state_err)?;
    let hash =
        tcc_state::json::Json::get(envelope.as_object().map_err(state_err)?, "artifact_hash")
            .map_err(state_err)?;
    Ok(hash.as_str().map_err(state_err)?.to_string())
}

fn encode_args(args: &[Value]) -> Result<String, HostError> {
    let mut items = Vec::with_capacity(args.len());
    for arg in args {
        items.push(tcc_state::json::Json::parse(&canonical(arg)?).map_err(state_err)?);
    }
    Ok(tcc_state::json::Json::Array(items).stringify())
}

fn patch_revision(body: &str, revision: u64) -> Result<String, HostError> {
    let mut json = tcc_state::json::Json::parse(body).map_err(state_err)?;
    match &mut json {
        tcc_state::json::Json::Object(map) => {
            map.insert(
                "revision".to_string(),
                tcc_state::json::Json::Number(revision as f64),
            );
        }
        _ => {
            return Err(HostError::Message(
                "continuation snapshot must be an object".to_string(),
            ))
        }
    }
    Ok(json.stringify())
}

fn status_name(status: ContinuationStatus) -> &'static str {
    match status {
        ContinuationStatus::Runnable => "runnable",
        ContinuationStatus::Running => "running",
        ContinuationStatus::Suspended => "suspended",
        ContinuationStatus::Completed => "completed",
        ContinuationStatus::Failed => "failed",
        ContinuationStatus::Cancelled => "cancelled",
    }
}

fn utf8(bytes: Vec<u8>) -> Result<String, HostError> {
    String::from_utf8(bytes).map_err(|err| HostError::Message(err.to_string()))
}

fn store_err(err: crate::store::StoreError) -> HostError {
    HostError::Message(err.message)
}

fn state_err(err: tcc_state::StateError) -> HostError {
    HostError::Message(err.to_string())
}

pub fn core_err(err: impl ToString) -> HostError {
    HostError::Message(err.to_string())
}
