use tcc_core::{CoreError, Engine};
use tcc_ir::{decode_artifact, encode_artifact, Artifact, EngineCaps, Instruction, IrError};
use tcc_state::Continuation;

#[test]
fn malformed_artifact_json_does_not_panic() {
    let result = std::panic::catch_unwind(|| decode_artifact("{\"envelope\":"));
    assert!(result.is_ok());
    assert!(result.unwrap().is_err());
}

#[test]
fn engine_start_returns_error_for_oob_local() {
    let mut artifact = Artifact::minimal_return("oob");
    artifact.program.functions[0].instructions = vec![
        Instruction::LoadLocal {
            local: tcc_ir::LocalId(3),
        },
        Instruction::Return,
    ];
    artifact.program.functions[0].spans = vec![None, None];
    artifact.program.functions[0].local_count = 1;
    let err = Engine::start(artifact, "oob", &EngineCaps::current()).unwrap_err();
    assert!(matches!(
        err,
        CoreError::Ir(IrError::LocalOutOfRange {
            local: 3,
            count: 1,
            ..
        })
    ));
}

#[test]
fn engine_resume_rejects_local_len_mismatch() {
    let artifact = Artifact::minimal_return("resume-locals");
    let json = encode_artifact(&artifact).unwrap();
    let artifact = decode_artifact(&json).unwrap();
    let mut continuation = Continuation::start(
        "exec",
        artifact.envelope.artifact_hash.clone(),
        artifact.envelope.engine_format_version,
        artifact.envelope.language_semantics_version.clone(),
        0,
        3,
    );
    continuation.frames[0].pc = 0;
    let err = Engine::resume(artifact, continuation, &EngineCaps::current()).unwrap_err();
    assert!(matches!(err, CoreError::InvalidContinuation(_)));
}
