// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! Owned Rust subset lowerer.
//!
//! `cargo check` against `tcc-rust-prelude` owns move, type, and exhaustiveness
//! checking. This crate walks the same source with `syn` and emits the existing
//! TCC instruction set. It does not call rustc.

mod lower;

use sha2::{Digest, Sha256};
use tcc_ir::Artifact;

pub use lower::CompileError;

/// npm package version stamped on every artifact as `frontend_version`.
pub const PACKAGE_VERSION: &str = "26.10.1";

/// Compile one Rust subset file into an artifact. `language_semantics_version` is `rust.subset.v1`.
pub fn compile(source: &str) -> Result<Artifact, CompileError> {
    let mut artifact = lower::lower(source)?;
    artifact.envelope.artifact_hash.clear();
    let canonical = tcc_ir::canonical_artifact_json(&artifact);
    let digest = Sha256::digest(canonical.as_bytes());
    artifact.envelope.artifact_hash = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(artifact)
}
