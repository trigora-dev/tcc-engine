use tcc_state::{ContinuationDelta, PersistKind, Value};

/// Host request/response JSON version. Required on every encoded message.
pub const HOST_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectStatus {
    Started,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectRecord {
    pub key: String,
    pub idempotency_key: String,
    pub status: EffectStatus,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitRegistration {
    pub wait_id: String,
    pub event_name: String,
    pub correlation_key: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChildSpec {
    pub invoke_id: String,
    pub child_execution_id: String,
    pub program_name: String,
    pub input: Option<Value>,
}

/// Coarse host participation. Not issued per instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum HostRequest {
    PersistCheckpoint {
        revision: u64,
        kind: PersistKind,
        base_revision: u64,
        materialize: bool,
        delta: Option<ContinuationDelta>,
    },
    RunEffect {
        key: String,
        idempotency_key: String,
    },
    PersistEffect {
        record: EffectRecord,
    },
    RegisterTimer {
        resume_at_ms: u64,
        branch: Option<String>,
    },
    RegisterWait {
        wait: WaitRegistration,
    },
    CreateChild {
        child: ChildSpec,
    },
    FetchArtifact {
        hash: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostResponse {
    Ack,
    PersistConfirmed {
        revision: u64,
    },
    EffectResult {
        value: Value,
    },
    EffectFailed {
        message: String,
    },
    EventPayload {
        value: Value,
        branch: Option<String>,
    },
    TimerFired {
        branch: Option<String>,
    },
    ChildResult {
        value: Value,
        branch: Option<String>,
    },
    Cancel,
    Artifact {
        hash: String,
    },
}

impl HostResponse {
    pub fn kind_name(&self) -> &'static str {
        match self {
            HostResponse::Ack => "ack",
            HostResponse::PersistConfirmed { .. } => "persist_confirmed",
            HostResponse::EffectResult { .. } => "effect_result",
            HostResponse::EffectFailed { .. } => "effect_failed",
            HostResponse::EventPayload { .. } => "event_payload",
            HostResponse::TimerFired { .. } => "timer_fired",
            HostResponse::ChildResult { .. } => "child_result",
            HostResponse::Cancel => "cancel",
            HostResponse::Artifact { .. } => "artifact",
        }
    }
}

impl HostRequest {
    pub fn kind_name(&self) -> &'static str {
        match self {
            HostRequest::PersistCheckpoint { .. } => "persist_checkpoint",
            HostRequest::RunEffect { .. } => "run_effect",
            HostRequest::PersistEffect { .. } => "persist_effect",
            HostRequest::RegisterTimer { .. } => "register_timer",
            HostRequest::RegisterWait { .. } => "register_wait",
            HostRequest::CreateChild { .. } => "create_child",
            HostRequest::FetchArtifact { .. } => "fetch_artifact",
        }
    }
}
