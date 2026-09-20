//! TCC program representation and versioned artifact envelope.
//!
//! This is the engine-consumable program contract, not a public product IR.
//! Frontends emit this format; `tcc-core` executes it.

#![allow(clippy::derive_partial_eq_without_eq)]

pub mod artifact;
pub mod encode;
pub mod error;
pub mod features;
pub mod instruction;
mod json;
pub mod liveness;
pub mod validate;

pub use artifact::{
    Artifact, Envelope, FuncId, Function, LocalId, Pc, Program, RuntimeModule, SourceSpan,
};
pub use encode::{canonical_artifact_json, decode_artifact, encode_artifact};
pub use error::IrError;
pub use features::{
    EngineFeature, FeatureSet, HostCapability, ENGINE_FORMAT_VERSION, FRONTEND_PYTHON,
    FRONTEND_TYPESCRIPT, LANGUAGE_SEMANTICS_PY, LANGUAGE_SEMANTICS_TS,
};
pub use instruction::{ConstValue, Instruction};
pub use liveness::{analyze_function, analyze_program, FunctionLiveness, LiveSet};
pub use validate::{validate, EngineCaps};
