use crate::artifact::{Artifact, FuncId, Pc};
use crate::error::IrError;
use crate::features::{EngineFeature, FeatureSet, HostCapability, ENGINE_FORMAT_VERSION};
use crate::instruction::Instruction;

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

    let mut seen = Vec::new();
    for function in &artifact.program.functions {
        if seen.contains(&function.id.0) {
            return Err(IrError::DuplicateFunction(function.id.0));
        }
        seen.push(function.id.0);

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
}
