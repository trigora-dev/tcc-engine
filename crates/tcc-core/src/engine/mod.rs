// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

mod calls;
mod collections;
mod concurrency;
mod durable;
mod execution;
mod host;
mod lifecycle;
mod persistence;

pub(super) use std::collections::HashMap;

pub(super) use tcc_ir::{
    analyze_program, validate, Artifact, ConstValue, EngineCaps, FuncId, FunctionLiveness,
    Instruction, Pc, ENGINE_FORMAT_VERSION, LANGUAGE_SEMANTICS_PY, LANGUAGE_SEMANTICS_RUST,
    LANGUAGE_SEMANTICS_TS, MAX_JOIN_BRANCHES,
};
pub(super) use tcc_state::{
    absorb_continuation, absorb_value, export_value, gc_heap, persist_intent, structural_eq,
    BranchOp, BranchPhase, Continuation, ContinuationStatus, HeapCell, JoinBranch, JoinKind,
    JoinReentry, JoinState, JoinStatus, PendingOp, PersistKind, Value, WaitKind,
};

pub(super) use crate::error::CoreError;
pub(super) use crate::protocol::{
    ChildSpec, EffectRecord, EffectStatus, HostRequest, HostResponse, WaitRegistration,
};

#[derive(Debug, Clone, PartialEq)]
pub enum EngineOutcome {
    Host(HostRequest),
    Completed { result: Value },
    Failed { message: String },
    Cancelled,
    BudgetExhausted,
    Suspended,
}

#[derive(Debug)]
pub struct Engine {
    artifact: Artifact,
    continuation: Continuation,
    outstanding: Option<HostRequest>,
    /// Last continuation confirmed by `persist_confirmed`. Unacked persists do not update this.
    confirmed: Option<Continuation>,
    deltas_since_snapshot: u32,
    liveness: HashMap<FuncId, FunctionLiveness>,
}

impl Engine {
    pub fn continuation(&self) -> &Continuation {
        &self.continuation
    }

    pub fn artifact_hash(&self) -> &str {
        &self.artifact.envelope.artifact_hash
    }
}
