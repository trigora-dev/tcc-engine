/// Engine-consumable artifact format version. Bump when the envelope or instruction
/// encoding changes in a way that existing engines cannot execute.
pub const ENGINE_FORMAT_VERSION: u32 = 1;

/// First-party TypeScript frontend identity.
pub const FRONTEND_TYPESCRIPT: &str = "typescript";

/// First-party Python frontend identity.
pub const FRONTEND_PYTHON: &str = "python";

/// Language-semantics version for the implemented TypeScript subset.
pub const LANGUAGE_SEMANTICS_TS: &str = "ts.subset.v1";

/// Language-semantics version for the implemented Python subset.
pub const LANGUAGE_SEMANTICS_PY: &str = "py.subset.v1";

pub fn known_language_semantics() -> &'static [&'static str] {
    &[LANGUAGE_SEMANTICS_TS, LANGUAGE_SEMANTICS_PY]
}

pub fn is_known_language_semantics(id: &str) -> bool {
    known_language_semantics().contains(&id)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EngineFeature(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostCapability(pub String);

impl EngineFeature {
    pub const TS_CONTROL_FLOW: &'static str = "ts.control_flow";
    pub const DURABLE_EFFECT: &'static str = "durable.effect";
    pub const DURABLE_SLEEP: &'static str = "durable.sleep";
    pub const DURABLE_WAIT_FOR_EVENT: &'static str = "durable.wait_for_event";
    pub const DURABLE_INVOKE: &'static str = "durable.invoke";
    pub const DURABLE_CONCURRENT_GROUP: &'static str = "durable.concurrent_group";
    pub const EXCEPTIONS: &'static str = "ts.exceptions";
    pub const LANG_COMPUTE: &'static str = "lang.compute";

    pub fn known() -> &'static [&'static str] {
        &[
            Self::TS_CONTROL_FLOW,
            Self::DURABLE_EFFECT,
            Self::DURABLE_SLEEP,
            Self::DURABLE_WAIT_FOR_EVENT,
            Self::DURABLE_INVOKE,
            Self::DURABLE_CONCURRENT_GROUP,
            Self::EXCEPTIONS,
            Self::LANG_COMPUTE,
        ]
    }

    pub fn is_known(&self) -> bool {
        Self::known().iter().any(|id| *id == self.0)
    }
}

impl HostCapability {
    pub const PERSIST_CHECKPOINT: &'static str = "host.persist_checkpoint";
    pub const PERSIST_CHECKPOINT_DELTA: &'static str = "host.persist_checkpoint_delta";
    pub const EFFECT: &'static str = "host.effect";
    pub const TIMER: &'static str = "host.timer";
    pub const EVENT: &'static str = "host.event";
    pub const CHILD: &'static str = "host.child";
    pub const FETCH_ARTIFACT: &'static str = "host.fetch_artifact";

    pub fn known() -> &'static [&'static str] {
        &[
            Self::PERSIST_CHECKPOINT,
            Self::PERSIST_CHECKPOINT_DELTA,
            Self::EFFECT,
            Self::TIMER,
            Self::EVENT,
            Self::CHILD,
            Self::FETCH_ARTIFACT,
        ]
    }

    pub fn is_known(&self) -> bool {
        Self::known().iter().any(|id| *id == self.0)
    }
}

/// Features and capabilities an engine or host currently implements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureSet {
    pub engine: Vec<EngineFeature>,
    pub host: Vec<HostCapability>,
}

impl FeatureSet {
    pub fn current_engine() -> Vec<EngineFeature> {
        EngineFeature::known()
            .iter()
            .map(|id| EngineFeature((*id).to_string()))
            .collect()
    }
}
