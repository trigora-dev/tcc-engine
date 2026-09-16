use tcc_state::Value;

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
    pub flow_name: String,
    pub input: Option<Value>,
}

/// Coarse host participation. Not issued per instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum HostRequest {
    PersistCheckpoint {
        revision: u64,
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
    PersistConfirmed { revision: u64 },
    EffectResult { value: Value },
    EffectFailed { message: String },
    EventPayload { value: Value },
    ChildResult { value: Value },
    Artifact { hash: String },
}

impl HostResponse {
    pub fn kind_name(&self) -> &'static str {
        match self {
            HostResponse::Ack => "ack",
            HostResponse::PersistConfirmed { .. } => "persist_confirmed",
            HostResponse::EffectResult { .. } => "effect_result",
            HostResponse::EffectFailed { .. } => "effect_failed",
            HostResponse::EventPayload { .. } => "event_payload",
            HostResponse::ChildResult { .. } => "child_result",
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
