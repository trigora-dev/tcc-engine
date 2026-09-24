use tcc_core::{CoreError, Engine};
use tcc_ir::ENGINE_FORMAT_VERSION;

use super::caps;

#[test]
fn resume_rejects_artifact_mismatch() {
    let artifact = tcc_ir::Artifact::minimal_return("hash-a");
    let engine = Engine::start(artifact, "exec-a", &caps()).unwrap();
    let continuation = engine.continuation().clone();
    let other = tcc_ir::Artifact::minimal_return("hash-b");
    let err = Engine::resume(other, continuation, &caps()).unwrap_err();
    assert!(matches!(err, CoreError::ArtifactMismatch { .. }));
}

#[test]
fn resume_rejects_pc_out_of_range() {
    let artifact = tcc_ir::Artifact::minimal_return("hash-pc");
    let mut continuation = tcc_state::Continuation::start(
        "exec-pc",
        artifact.envelope.artifact_hash.clone(),
        ENGINE_FORMAT_VERSION,
        artifact.envelope.language_semantics_version.clone(),
        0,
        0,
    );
    continuation.frames[0].pc = 99;
    let err = Engine::resume(artifact, continuation, &caps()).unwrap_err();
    assert!(matches!(err, CoreError::InvalidContinuation(message) if message.contains("pc")));
}

#[test]
fn resume_rejects_language_semantics_mismatch() {
    let artifact = tcc_ir::Artifact::minimal_return("hash-sem");
    let continuation = tcc_state::Continuation::start(
        "exec-sem",
        artifact.envelope.artifact_hash.clone(),
        ENGINE_FORMAT_VERSION,
        "py.subset.v1",
        0,
        0,
    );
    let err = Engine::resume(artifact, continuation, &caps()).unwrap_err();
    assert!(matches!(err, CoreError::LanguageSemanticsMismatch { .. }));
}
