use crate::value::Value;

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
        }
    }
}
