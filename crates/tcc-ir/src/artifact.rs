// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use crate::features::{EngineFeature, HostCapability, ENGINE_FORMAT_VERSION};
use crate::instruction::{ConstValue, Instruction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pc(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    pub file: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeModule {
    pub id: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub artifact_hash: String,
    pub frontend_id: String,
    pub frontend_version: String,
    pub language_semantics_version: String,
    pub engine_format_version: u32,
    pub required_engine_features: Vec<EngineFeature>,
    pub required_host_capabilities: Vec<HostCapability>,
    pub runtime_modules: Vec<RuntimeModule>,
}

impl Envelope {
    pub fn typescript_v1(artifact_hash: impl Into<String>) -> Self {
        Self {
            artifact_hash: artifact_hash.into(),
            frontend_id: crate::FRONTEND_TYPESCRIPT.to_string(),
            frontend_version: "0.0.0".to_string(),
            language_semantics_version: crate::LANGUAGE_SEMANTICS_TS.to_string(),
            engine_format_version: ENGINE_FORMAT_VERSION,
            required_engine_features: vec![EngineFeature(
                EngineFeature::TS_CONTROL_FLOW.to_string(),
            )],
            required_host_capabilities: vec![HostCapability(
                HostCapability::PERSIST_CHECKPOINT.to_string(),
            )],
            runtime_modules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub id: FuncId,
    pub name: String,
    pub param_count: u32,
    pub local_count: u32,
    /// Aligned with parameters. `None` means no default. Omitted when empty.
    pub param_defaults: Vec<Option<ConstValue>>,
    pub instructions: Vec<Instruction>,
    /// Parallel to `instructions`. Missing entries are allowed as `None`.
    pub spans: Vec<Option<SourceSpan>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub entry: FuncId,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    pub envelope: Envelope,
    pub program: Program,
}

impl Artifact {
    pub fn function(&self, id: FuncId) -> Option<&Function> {
        self.program
            .functions
            .iter()
            .find(|function| function.id == id)
    }
}
