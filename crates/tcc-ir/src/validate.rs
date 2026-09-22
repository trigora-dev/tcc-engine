use crate::artifact::{Artifact, FuncId, Pc};
use crate::error::IrError;
use crate::features::{
    is_known_language_semantics, EngineFeature, FeatureSet, HostCapability, ENGINE_FORMAT_VERSION,
};
use crate::instruction::Instruction;

/// Conservative limits for untrusted artifacts. Documented in spec/program-format.md.
pub const MAX_FUNCTIONS: usize = 1_024;
pub const MAX_INSTRUCTIONS_PER_FUNCTION: usize = 100_000;
pub const MAX_LOCALS: u32 = 4_096;
pub const MAX_STRING_BYTES: usize = 1_048_576;
pub const MAX_JOIN_BRANCHES: usize = 32;

/// What this engine build can execute and what a host is willing to provide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineCaps {
    pub features: FeatureSet,
}

impl EngineCaps {
    pub fn current() -> Self {
        Self {
            features: FeatureSet {
                engine: FeatureSet::current_engine(),
                host: HostCapability::known()
                    .iter()
                    .map(|id| HostCapability((*id).to_string()))
                    .collect(),
            },
        }
    }

    pub fn supports_engine(&self, feature: &EngineFeature) -> bool {
        self.features
            .engine
            .iter()
            .any(|known| known.0 == feature.0)
    }

    pub fn supports_host(&self, capability: &HostCapability) -> bool {
        self.features
            .host
            .iter()
            .any(|known| known.0 == capability.0)
    }
}

pub fn validate(artifact: &Artifact, caps: &EngineCaps) -> Result<(), IrError> {
    if artifact.envelope.engine_format_version != ENGINE_FORMAT_VERSION {
        return Err(IrError::UnsupportedFormat {
            found: artifact.envelope.engine_format_version,
            supported: ENGINE_FORMAT_VERSION,
        });
    }

    if artifact.envelope.artifact_hash.is_empty() {
        return Err(IrError::EmptyArtifactHash);
    }

    if artifact.envelope.frontend_id.is_empty() {
        return Err(IrError::EmptyFrontendId);
    }

    if !is_known_language_semantics(&artifact.envelope.language_semantics_version) {
        return Err(IrError::UnsupportedLanguageSemantics(
            artifact.envelope.language_semantics_version.clone(),
        ));
    }

    for feature in &artifact.envelope.required_engine_features {
        if !feature.is_known() || !caps.supports_engine(feature) {
            return Err(IrError::UnsupportedEngineFeature(feature.0.clone()));
        }
    }

    for capability in &artifact.envelope.required_host_capabilities {
        if !capability.is_known() || !caps.supports_host(capability) {
            return Err(IrError::UnsupportedHostCapability(capability.0.clone()));
        }
    }

    if artifact.program.functions.is_empty() {
        return Err(IrError::EmptyProgram);
    }

    if artifact.program.functions.len() > MAX_FUNCTIONS {
        return Err(IrError::LimitsExceeded {
            what: "function count",
            got: artifact.program.functions.len(),
            max: MAX_FUNCTIONS,
        });
    }

    let mut seen = Vec::new();
    for function in &artifact.program.functions {
        if seen.contains(&function.id.0) {
            return Err(IrError::DuplicateFunction(function.id.0));
        }
        seen.push(function.id.0);

        if function.local_count > MAX_LOCALS {
            return Err(IrError::LimitsExceeded {
                what: "local_count",
                got: function.local_count as usize,
                max: MAX_LOCALS as usize,
            });
        }

        if function.instructions.len() > MAX_INSTRUCTIONS_PER_FUNCTION {
            return Err(IrError::LimitsExceeded {
                what: "instruction count",
                got: function.instructions.len(),
                max: MAX_INSTRUCTIONS_PER_FUNCTION,
            });
        }

        if function.spans.len() != function.instructions.len() {
            return Err(IrError::SpanLengthMismatch {
                func: function.id.0,
                index: function.spans.len(),
            });
        }

        let len = function.instructions.len();
        for instruction in &function.instructions {
            for Pc(target) in instruction.jump_targets() {
                if target as usize >= len {
                    return Err(IrError::JumpOutOfRange {
                        func: function.id.0,
                        target,
                        len,
                    });
                }
            }
            if let Instruction::Call { func, .. } = instruction {
                if artifact.function(*func).is_none() {
                    return Err(IrError::MissingEntry(func.0));
                }
            }
            if let Some(local) = instruction.uses_local().or(instruction.defs_local()) {
                if local.0 >= function.local_count {
                    return Err(IrError::LocalOutOfRange {
                        func: function.id.0,
                        local: local.0,
                        count: function.local_count,
                    });
                }
            }
            if let Instruction::Fork { count, .. } = instruction {
                if *count == 0 || *count as usize > MAX_JOIN_BRANCHES {
                    return Err(IrError::LimitsExceeded {
                        what: "join branches",
                        got: *count as usize,
                        max: MAX_JOIN_BRANCHES,
                    });
                }
                if !artifact
                    .envelope
                    .required_engine_features
                    .iter()
                    .any(|feature| feature.0 == EngineFeature::DURABLE_CONCURRENT_GROUP)
                {
                    return Err(IrError::InvalidEncoding(
                        "Fork requires durable.concurrent_group".into(),
                    ));
                }
            }
            if matches!(instruction, Instruction::JoinAll | Instruction::JoinAny)
                && !artifact
                    .envelope
                    .required_engine_features
                    .iter()
                    .any(|feature| feature.0 == EngineFeature::DURABLE_CONCURRENT_GROUP)
            {
                return Err(IrError::InvalidEncoding(
                    "JoinAll or JoinAny requires durable.concurrent_group".into(),
                ));
            }
            match instruction {
                Instruction::LoadConst {
                    value: crate::instruction::ConstValue::String(text),
                } if text.len() > MAX_STRING_BYTES => {
                    return Err(IrError::LimitsExceeded {
                        what: "string length",
                        got: text.len(),
                        max: MAX_STRING_BYTES,
                    });
                }
                Instruction::SetProp { key } | Instruction::GetProp { key }
                    if key.len() > MAX_STRING_BYTES =>
                {
                    return Err(IrError::LimitsExceeded {
                        what: "key length",
                        got: key.len(),
                        max: MAX_STRING_BYTES,
                    });
                }
                _ => {}
            }
        }
    }

    if artifact.function(artifact.program.entry).is_none() {
        return Err(IrError::MissingEntry(artifact.program.entry.0));
    }

    Ok(())
}

impl Artifact {
    pub fn minimal_return(artifact_hash: impl Into<String>) -> Self {
        use crate::artifact::{Envelope, Function, Program};

        Self {
            envelope: Envelope::typescript_v1(artifact_hash),
            program: Program {
                entry: FuncId(0),
                functions: vec![Function {
                    id: FuncId(0),
                    name: "main".to_string(),
                    param_count: 0,
                    local_count: 0,
                    instructions: vec![Instruction::Return],
                    spans: vec![None],
                }],
            },
        }
    }

    pub fn sdk_first_example(artifact_hash: impl Into<String>) -> Self {
        use crate::artifact::{Envelope, Function, LocalId, Program};
        use crate::instruction::ConstValue;

        Self {
            envelope: Envelope {
                artifact_hash: artifact_hash.into(),
                frontend_id: crate::FRONTEND_TYPESCRIPT.to_string(),
                frontend_version: "0.0.0".to_string(),
                language_semantics_version: crate::LANGUAGE_SEMANTICS_TS.to_string(),
                engine_format_version: ENGINE_FORMAT_VERSION,
                required_engine_features: vec![
                    EngineFeature(EngineFeature::TS_CONTROL_FLOW.to_string()),
                    EngineFeature(EngineFeature::DURABLE_EFFECT.to_string()),
                    EngineFeature(EngineFeature::DURABLE_WAIT_FOR_EVENT.to_string()),
                ],
                required_host_capabilities: vec![
                    HostCapability(HostCapability::PERSIST_CHECKPOINT.to_string()),
                    HostCapability(HostCapability::EFFECT.to_string()),
                    HostCapability(HostCapability::EVENT.to_string()),
                ],
                runtime_modules: Vec::new(),
            },
            program: Program {
                entry: FuncId(0),
                functions: vec![Function {
                    id: FuncId(0),
                    name: "run".to_string(),
                    param_count: 0,
                    local_count: 2,
                    instructions: vec![
                        Instruction::LoadConst {
                            value: ConstValue::String("generate".to_string()),
                        },
                        Instruction::Effect,
                        Instruction::StoreLocal { local: LocalId(0) },
                        Instruction::LoadConst {
                            value: ConstValue::String("approved".to_string()),
                        },
                        Instruction::WaitForEvent,
                        Instruction::StoreLocal { local: LocalId(1) },
                        Instruction::NewObject,
                        Instruction::LoadLocal { local: LocalId(0) },
                        Instruction::SetProp {
                            key: "result".to_string(),
                        },
                        Instruction::LoadLocal { local: LocalId(1) },
                        Instruction::SetProp {
                            key: "approval".to_string(),
                        },
                        Instruction::Return,
                    ],
                    spans: vec![None; 12],
                }],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{Envelope, Function, Program};
    use crate::instruction::Instruction;

    #[test]
    fn rejects_unknown_required_feature() {
        let mut artifact = Artifact::minimal_return("abc");
        artifact
            .envelope
            .required_engine_features
            .push(EngineFeature("python.int".to_string()));
        let err = validate(&artifact, &EngineCaps::current()).unwrap_err();
        assert!(matches!(err, IrError::UnsupportedEngineFeature(id) if id == "python.int"));
    }

    #[test]
    fn rejects_format_mismatch() {
        let mut artifact = Artifact::minimal_return("abc");
        artifact.envelope.engine_format_version = 99;
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::UnsupportedFormat { found: 99, .. })
        ));
    }

    #[test]
    fn accepts_minimal_typescript_program() {
        validate(&Artifact::minimal_return("abc"), &EngineCaps::current()).unwrap();
    }

    #[test]
    fn rejects_jump_past_end() {
        let artifact = Artifact {
            envelope: Envelope::typescript_v1("abc"),
            program: Program {
                entry: FuncId(0),
                functions: vec![Function {
                    id: FuncId(0),
                    name: "main".to_string(),
                    param_count: 0,
                    local_count: 0,
                    instructions: vec![Instruction::Jump { target: Pc(4) }, Instruction::Return],
                    spans: vec![None, None],
                }],
            },
        };
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::JumpOutOfRange { target: 4, .. })
        ));
    }

    #[test]
    fn rejects_empty_artifact_hash() {
        let mut artifact = Artifact::minimal_return("abc");
        artifact.envelope.artifact_hash.clear();
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::EmptyArtifactHash)
        ));
    }

    #[test]
    fn rejects_empty_program() {
        let artifact = Artifact {
            envelope: Envelope::typescript_v1("abc"),
            program: Program {
                entry: FuncId(0),
                functions: vec![],
            },
        };
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::EmptyProgram)
        ));
    }

    #[test]
    fn rejects_duplicate_function() {
        let artifact = Artifact {
            envelope: Envelope::typescript_v1("abc"),
            program: Program {
                entry: FuncId(0),
                functions: vec![
                    Function {
                        id: FuncId(0),
                        name: "a".to_string(),
                        param_count: 0,
                        local_count: 0,
                        instructions: vec![Instruction::Return],
                        spans: vec![None],
                    },
                    Function {
                        id: FuncId(0),
                        name: "b".to_string(),
                        param_count: 0,
                        local_count: 0,
                        instructions: vec![Instruction::Return],
                        spans: vec![None],
                    },
                ],
            },
        };
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::DuplicateFunction(0))
        ));
    }

    #[test]
    fn rejects_span_length_mismatch() {
        let artifact = Artifact {
            envelope: Envelope::typescript_v1("abc"),
            program: Program {
                entry: FuncId(0),
                functions: vec![Function {
                    id: FuncId(0),
                    name: "main".to_string(),
                    param_count: 0,
                    local_count: 0,
                    instructions: vec![Instruction::Return],
                    spans: vec![],
                }],
            },
        };
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::SpanLengthMismatch { func: 0, index: 0 })
        ));
    }

    #[test]
    fn rejects_unsupported_host_capability() {
        let mut artifact = Artifact::minimal_return("abc");
        artifact
            .envelope
            .required_host_capabilities
            .push(HostCapability("host.made_up".to_string()));
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::UnsupportedHostCapability(id)) if id == "host.made_up"
        ));
    }

    #[test]
    fn rejects_empty_frontend_id() {
        let mut artifact = Artifact::minimal_return("abc");
        artifact.envelope.frontend_id.clear();
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::EmptyFrontendId)
        ));
    }

    #[test]
    fn rejects_unknown_language_semantics() {
        let mut artifact = Artifact::minimal_return("abc");
        artifact.envelope.language_semantics_version = "js.full.v1".into();
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::UnsupportedLanguageSemantics(id)) if id == "js.full.v1"
        ));
    }

    #[test]
    fn rejects_local_out_of_range() {
        let mut artifact = Artifact::minimal_return("abc");
        artifact.program.functions[0].instructions = vec![
            Instruction::LoadLocal {
                local: crate::artifact::LocalId(0),
            },
            Instruction::Return,
        ];
        artifact.program.functions[0].spans = vec![None, None];
        artifact.program.functions[0].local_count = 0;
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::LocalOutOfRange {
                func: 0,
                local: 0,
                count: 0
            })
        ));
    }

    #[test]
    fn rejects_too_many_locals() {
        let mut artifact = Artifact::minimal_return("abc");
        artifact.program.functions[0].local_count = MAX_LOCALS + 1;
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::LimitsExceeded {
                what: "local_count",
                ..
            })
        ));
    }

    #[test]
    fn rejects_oversized_const_string() {
        let mut artifact = Artifact::minimal_return("abc");
        artifact.program.functions[0].instructions = vec![
            Instruction::LoadConst {
                value: crate::instruction::ConstValue::String("x".repeat(MAX_STRING_BYTES + 1)),
            },
            Instruction::Return,
        ];
        artifact.program.functions[0].spans = vec![None, None];
        assert!(matches!(
            validate(&artifact, &EngineCaps::current()),
            Err(IrError::LimitsExceeded {
                what: "string length",
                ..
            })
        ));
    }
}
