// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use std::path::Path;
use std::time::Instant;

use tcc_core::{decode_args_array, Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_host::{Host, HostError};
use tcc_ir::{decode_artifact, Artifact, EngineCaps};
use tcc_state::{
    decode_continuation, decode_value, BranchOp, BranchPhase, JoinState, JoinStatus, PendingOp,
    Value, WaitKind,
};

use crate::host::{canonical, core_err, SqliteHost};
use crate::store::StoreError;
use crate::trace::{HostTraceEvent, HostTraceKind};

#[derive(Debug, Clone, PartialEq)]
pub struct RunResult {
    pub status: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub continuation_json: Option<String>,
    pub revision: u64,
}

const BUDGET: u32 = 256;

pub fn start_execution(
    host: &mut SqliteHost,
    artifact_json: &str,
    execution_id: &str,
) -> Result<RunResult, HostError> {
    start_execution_with_args(host, artifact_json, execution_id, &[])
}

pub fn start_execution_with_args(
    host: &mut SqliteHost,
    artifact_json: &str,
    execution_id: &str,
    args: &[Value],
) -> Result<RunResult, HostError> {
    let artifact = decode_artifact(artifact_json).map_err(core_err)?;
    let hash = artifact.envelope.artifact_hash.clone();
    host.store
        .put_artifact(&hash, artifact_json)
        .map_err(store_err)?;
    host.store
        .create_execution(execution_id, &hash)
        .map_err(store_err)?;
    host.execution_id = execution_id.to_string();
    let engine = Engine::start_with_args(artifact, execution_id, &EngineCaps::current(), args)
        .map_err(core_err)?;
    drive(host, engine)
}

pub fn resume_execution(
    host: &mut SqliteHost,
    artifact_json: Option<&str>,
    execution_id: &str,
) -> Result<RunResult, HostError> {
    let execution = host
        .store
        .execution(execution_id)
        .map_err(store_err)?
        .ok_or_else(|| HostError::Message(format!("unknown execution `{execution_id}`")))?;
    let artifact_json = match artifact_json {
        Some(text) => text.to_string(),
        None => host
            .store
            .artifact(&execution.artifact_hash)
            .map_err(store_err)?
            .ok_or_else(|| {
                HostError::Message(format!("missing artifact `{}`", execution.artifact_hash))
            })?,
    };
    let artifact = decode_artifact(&artifact_json).map_err(core_err)?;
    host.execution_id = execution_id.to_string();
    let caps = EngineCaps::current();
    let engine = if let Some(saved) = host.store.checkpoint(execution_id).map_err(store_err)? {
        restore_engine(host, artifact, &saved.body, saved.revision, &caps)?
    } else {
        Engine::start(artifact, execution_id, &caps).map_err(core_err)?
    };
    drive(host, engine)
}

fn restore_engine(
    host: &mut SqliteHost,
    artifact: Artifact,
    body: &str,
    revision: u64,
    caps: &EngineCaps,
) -> Result<Engine, HostError> {
    let started = Instant::now();
    let continuation = decode_continuation(body.as_bytes()).map_err(core_err)?;
    let engine = Engine::resume(artifact, continuation, caps).map_err(core_err)?;
    let duration = started.elapsed();
    let bytes = body.len();
    let retained = host.trace_continuations.then(|| body.to_string());
    host.emit_trace(HostTraceEvent {
        kind: HostTraceKind::ContinuationRestored,
        revision: Some(revision),
        duration,
        bytes: Some(bytes),
        body: retained,
        effect_key: None,
        journal_hit: false,
        suspended: false,
    });
    Ok(engine)
}

pub fn read_continuation(path: &Path, execution_id: &str) -> Result<String, HostError> {
    let host = SqliteHost::open(path, execution_id)?;
    host.store
        .checkpoint(execution_id)
        .map_err(store_err)?
        .map(|row| row.body)
        .ok_or_else(|| HostError::Message(format!("missing continuation for {execution_id}")))
}

fn drive(host: &mut SqliteHost, mut engine: Engine) -> Result<RunResult, HostError> {
    loop {
        match engine.run_until_host(BUDGET) {
            EngineOutcome::Host(request) => {
                if matches!(request, HostRequest::PersistCheckpoint { .. }) {
                    host.stage_continuation(engine.continuation())?;
                }
                let response = host.handle(request)?;
                engine.apply_host_response(response).map_err(core_err)?;
            }
            EngineOutcome::Completed { result } => {
                return finish(host, "completed", Some(result), None);
            }
            EngineOutcome::Failed { message } => {
                return finish(host, "failed", None, Some(message));
            }
            EngineOutcome::Cancelled => return finish(host, "cancelled", None, None),
            EngineOutcome::BudgetExhausted => {
                return Err(HostError::Message(
                    "instruction budget exhausted".to_string(),
                ));
            }
            EngineOutcome::Suspended => match deliver(host, &mut engine)? {
                Step::Continue => {}
                Step::Stop => return finish(host, "suspended", None, None),
            },
        }
    }
}

enum Step {
    Continue,
    Stop,
}

fn deliver(host: &mut SqliteHost, engine: &mut Engine) -> Result<Step, HostError> {
    if host.cancel {
        host.crash
            .hit("before_cancel", None)
            .map_err(|hit| HostError::Message(hit.to_string()))?;
        engine
            .apply_host_response(HostResponse::Cancel)
            .map_err(core_err)?;
        return Ok(Step::Continue);
    }
    let pending = engine.continuation().pending.clone();
    let join = engine.continuation().join.clone();
    if let Some(join) = join {
        if join.state == JoinStatus::Active && pending.is_none() {
            return deliver_join(host, engine, &join);
        }
    }
    match pending {
        Some(PendingOp::Wait {
            kind: WaitKind::Timer { .. },
        }) => deliver_timer(host, engine, None),
        Some(PendingOp::Wait {
            kind: WaitKind::Child { invoke_id, .. },
        }) => deliver_child(host, engine, &invoke_id, None),
        Some(PendingOp::Wait {
            kind: WaitKind::Event { wait_id, .. },
        }) => deliver_event(host, engine, &wait_id, false),
        _ => Ok(Step::Stop),
    }
}

fn deliver_join(
    host: &mut SqliteHost,
    engine: &mut Engine,
    join: &JoinState,
) -> Result<Step, HostError> {
    let mut branches: Vec<_> = join
        .branches
        .iter()
        .filter(|branch| branch.phase == BranchPhase::Registered)
        .cloned()
        .collect();
    branches.sort_by_key(|branch| branch.index);
    if host.completion_reverse {
        branches.reverse();
    }
    let Some(branch) = branches.into_iter().next() else {
        return Ok(Step::Stop);
    };
    match branch.op {
        Some(BranchOp::Timer { .. }) => deliver_timer(host, engine, Some(branch.branch_id)),
        Some(BranchOp::Child { .. }) => {
            let branch_id = branch.branch_id;
            deliver_child(host, engine, &branch_id, Some(branch_id.clone()))
        }
        Some(BranchOp::Event { .. }) | None => deliver_event(host, engine, &branch.branch_id, true),
        Some(BranchOp::Effect { .. }) => Err(HostError::Message(
            "effect branches are not wakes".to_string(),
        )),
    }
}

fn deliver_timer(
    host: &mut SqliteHost,
    engine: &mut Engine,
    branch: Option<String>,
) -> Result<Step, HostError> {
    let key = branch.clone().unwrap_or_default();
    let row = host
        .store
        .timer(&host.execution_id, &key)
        .map_err(store_err)?;
    let Some(row) = row else {
        return Ok(Step::Stop);
    };
    if row.status == "pending" {
        host.crash
            .hit("before_timer_fired", None)
            .map_err(|hit| HostError::Message(hit.to_string()))?;
        host.store
            .resolve_timer(&host.execution_id, &row.branch)
            .map_err(store_err)?;
    } else if row.status != "resolved" {
        return Ok(Step::Stop);
    }
    let response_branch = branch.filter(|value| !value.is_empty());
    engine
        .apply_host_response(HostResponse::TimerFired {
            branch: response_branch,
        })
        .map_err(core_err)?;
    Ok(Step::Continue)
}

fn deliver_event(
    host: &mut SqliteHost,
    engine: &mut Engine,
    wait_id: &str,
    join: bool,
) -> Result<Step, HostError> {
    let wait = host.store.wait(wait_id).map_err(store_err)?;
    let Some(wait) = wait else {
        return Ok(Step::Stop);
    };
    let payload = if wait.status == "pending" {
        host.crash
            .hit("before_event_payload", None)
            .map_err(|hit| HostError::Message(hit.to_string()))?;
        let payload = if let Some(queued) = host
            .store
            .take_event(&host.execution_id, &wait.event_name)
            .map_err(store_err)?
        {
            queued
        } else if host.auto_deliver {
            let fallback = Value::String("ok".to_string());
            canonical(host.event_payload.as_ref().unwrap_or(&fallback))?
        } else {
            return Ok(Step::Stop);
        };
        host.store
            .resolve_wait(&wait.id, &payload)
            .map_err(store_err)?;
        host.crash
            .hit("after_event_persist", None)
            .map_err(|hit| HostError::Message(hit.to_string()))?;
        payload
    } else if let Some(payload) = wait.payload {
        payload
    } else {
        return Ok(Step::Stop);
    };
    let value = decode_value(payload.as_bytes()).map_err(core_err)?;
    engine
        .apply_host_response(HostResponse::EventPayload {
            value,
            branch: join.then(|| wait_id.to_string()),
        })
        .map_err(core_err)?;
    Ok(Step::Continue)
}

fn deliver_child(
    host: &mut SqliteHost,
    engine: &mut Engine,
    invoke_id: &str,
    branch: Option<String>,
) -> Result<Step, HostError> {
    let child = host.store.child(invoke_id).map_err(store_err)?;
    let Some(child) = child else {
        return Ok(Step::Stop);
    };
    if child.status == "completed" {
        let Some(result_json) = child.result_json else {
            return Err(HostError::Message(format!(
                "child `{invoke_id}` completed without a result"
            )));
        };
        return apply_child_result(host, engine, &result_json, branch);
    }
    let artifact_json = host
        .store
        .artifact(&child.artifact_hash)
        .map_err(store_err)?
        .ok_or_else(|| HostError::Message(format!("missing artifact `{}`", child.artifact_hash)))?;
    let artifact = decode_artifact(&artifact_json).map_err(core_err)?;
    let args = match &child.args_json {
        Some(text) => decode_args_array(text).map_err(core_err)?,
        None => Vec::new(),
    };
    let child_id = child.child_execution_id.clone();
    if host
        .store
        .execution(&child_id)
        .map_err(store_err)?
        .is_none()
    {
        host.store
            .create_execution(&child_id, &child.artifact_hash)
            .map_err(store_err)?;
    }
    let caps = EngineCaps::current();
    let child_engine = if let Some(saved) = host.store.checkpoint(&child_id).map_err(store_err)? {
        restore_engine(host, artifact, &saved.body, saved.revision, &caps)?
    } else {
        Engine::start_with_args(artifact, &child_id, &caps, &args).map_err(core_err)?
    };
    let parent_id = std::mem::replace(&mut host.execution_id, child_id);
    let parent_cancel = std::mem::replace(&mut host.cancel, false);
    let child_result = drive(host, child_engine);
    host.execution_id = parent_id;
    host.cancel = parent_cancel;
    let child_result = child_result?;
    if child_result.status != "completed" {
        return Ok(Step::Stop);
    }
    let stored = host
        .store
        .child(invoke_id)
        .map_err(store_err)?
        .and_then(|row| row.result_json)
        .ok_or_else(|| HostError::Message(format!("child `{invoke_id}` has no stored result")))?;
    apply_child_result(host, engine, &stored, branch)
}

fn apply_child_result(
    host: &mut SqliteHost,
    engine: &mut Engine,
    result_json: &str,
    branch: Option<String>,
) -> Result<Step, HostError> {
    host.crash
        .hit("before_child_result", None)
        .map_err(|hit| HostError::Message(hit.to_string()))?;
    let value = decode_value(result_json.as_bytes()).map_err(core_err)?;
    engine
        .apply_host_response(HostResponse::ChildResult { value, branch })
        .map_err(core_err)?;
    Ok(Step::Continue)
}

fn finish(
    host: &SqliteHost,
    status: &str,
    result: Option<Value>,
    error: Option<String>,
) -> Result<RunResult, HostError> {
    let saved = host
        .store
        .checkpoint(&host.execution_id)
        .map_err(store_err)?;
    Ok(RunResult {
        status: status.to_string(),
        result,
        error,
        continuation_json: saved.as_ref().map(|row| row.body.clone()),
        revision: saved.as_ref().map(|row| row.revision).unwrap_or(0),
    })
}

fn store_err(err: StoreError) -> HostError {
    HostError::Message(err.message)
}
