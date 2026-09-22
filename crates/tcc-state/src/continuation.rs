use crate::value::{HeapCell, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactId {
    pub hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationStatus {
    Runnable,
    Running,
    Suspended,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub func_id: u32,
    pub pc: u32,
    pub locals: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryHandler {
    pub catch: u32,
    pub finally: Option<u32>,
    pub stack_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitKind {
    Timer {
        resume_at_ms: u64,
    },
    Event {
        wait_id: String,
        event_name: String,
        correlation_key: Option<String>,
    },
    Child {
        invoke_id: String,
        child_execution_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchPhase {
    Planned,
    Registered,
    Completed,
    Failed,
    Detached,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BranchOp {
    Effect {
        key: String,
    },
    Timer {
        resume_at_ms: u64,
    },
    Event {
        event_name: String,
    },
    Child {
        program_name: String,
        invoke_id: String,
        child_execution_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    All,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinStatus {
    Active,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinBranch {
    pub index: u32,
    pub branch_id: String,
    pub op: Option<BranchOp>,
    pub phase: BranchPhase,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinState {
    pub kind: JoinKind,
    pub site: u32,
    pub reentry: u32,
    pub join_pc: u32,
    pub state: JoinStatus,
    pub winner_branch: Option<u32>,
    pub failure_branch: Option<u32>,
    pub branches: Vec<JoinBranch>,
}

/// Next reentry to use when `Fork` at `site` runs again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinReentry {
    pub site: u32,
    pub next: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingOp {
    Effect {
        key: String,
        idempotency_key: String,
    },
    Wait {
        kind: WaitKind,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Continuation {
    pub execution_id: String,
    pub artifact: ArtifactId,
    pub engine_format_version: u32,
    pub language_semantics_version: String,
    pub revision: u64,
    pub status: ContinuationStatus,
    pub frames: Vec<Frame>,
    pub stack: Vec<Value>,
    pub pending: Option<PendingOp>,
    pub result: Option<Value>,
    pub try_stack: Vec<TryHandler>,
    pub join: Option<JoinState>,
    pub reentries: Vec<JoinReentry>,
    /// Object and array cells. Empty when the execution has not allocated.
    pub heap: Vec<HeapCell>,
    /// Heap ids whose structure must not change, outermost loop last.
    pub iterating: Vec<u32>,
}

impl Continuation {
    pub fn start(
        execution_id: impl Into<String>,
        artifact_hash: impl Into<String>,
        engine_format_version: u32,
        language_semantics_version: impl Into<String>,
        entry_func: u32,
        local_count: u32,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            artifact: ArtifactId {
                hash: artifact_hash.into(),
            },
            engine_format_version,
            language_semantics_version: language_semantics_version.into(),
            revision: 0,
            status: ContinuationStatus::Runnable,
            frames: vec![Frame {
                func_id: entry_func,
                pc: 0,
                locals: vec![Value::Undefined; local_count as usize],
            }],
            stack: Vec::new(),
            pending: None,
            result: None,
            try_stack: Vec::new(),
            join: None,
            reentries: Vec::new(),
            heap: Vec::new(),
            iterating: Vec::new(),
        }
    }
}
