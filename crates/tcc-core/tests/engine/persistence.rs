use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_state::ContinuationStatus;

use super::caps;

#[test]
fn persist_ack_does_not_commit_revision() {
    let artifact = tcc_ir::Artifact::minimal_return("hash-ack");
    let mut engine = Engine::start(artifact, "exec-ack", &caps()).unwrap();
    assert!(matches!(
        engine.run_until_host(16),
        EngineOutcome::Host(HostRequest::PersistCheckpoint { .. })
    ));
    engine.apply_host_response(HostResponse::Ack).unwrap();
    assert_eq!(engine.continuation().revision, 0);
    assert_eq!(engine.continuation().status, ContinuationStatus::Completed);
}

#[test]
fn persist_confirmed_sets_revision() {
    let artifact = tcc_ir::Artifact::minimal_return("hash-confirm");
    let mut engine = Engine::start(artifact, "exec-confirm", &caps()).unwrap();
    engine.run_until_host(16);
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision: 7 })
        .unwrap();
    assert_eq!(engine.continuation().revision, 7);
    assert_eq!(engine.continuation().status, ContinuationStatus::Completed);
}
