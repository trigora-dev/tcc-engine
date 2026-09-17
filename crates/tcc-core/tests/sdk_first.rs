use std::collections::BTreeMap;

use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{encode_artifact, Artifact, EngineCaps};
use tcc_state::{ContinuationStatus, Value};

fn expect_host(outcome: EngineOutcome) -> HostRequest {
    match outcome {
        EngineOutcome::Host(request) => request,
        other => panic!("expected host request, got {other:?}"),
    }
}

#[test]
fn sdk_first_example_completes_with_object_result() {
    let artifact = Artifact::sdk_first_example("sdk-first");
    let mut engine = Engine::start(artifact, "first", &EngineCaps::current()).unwrap();

    assert!(matches!(
        expect_host(engine.run_until_host(32)),
        HostRequest::RunEffect { ref key, .. } if key == "generate"
    ));
    engine
        .apply_host_response(HostResponse::EffectResult {
            value: Value::Number(42.0),
        })
        .unwrap();
    expect_host(engine.run_until_host(32));
    engine.apply_host_response(HostResponse::Ack).unwrap();
    expect_host(engine.run_until_host(32));
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision: 1 })
        .unwrap();

    match expect_host(engine.run_until_host(32)) {
        HostRequest::RegisterWait { wait } => {
            assert_eq!(wait.wait_id, "first:approved::4");
        }
        other => panic!("expected register_wait, got {other:?}"),
    }
    engine.apply_host_response(HostResponse::Ack).unwrap();
    expect_host(engine.run_until_host(32));
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision: 2 })
        .unwrap();
    assert_eq!(engine.run_until_host(32), EngineOutcome::Suspended);

    engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("ok".into()),
        })
        .unwrap();
    expect_host(engine.run_until_host(32));
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision: 3 })
        .unwrap();
    expect_host(engine.run_until_host(32));
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision: 4 })
        .unwrap();

    assert_eq!(engine.continuation().status, ContinuationStatus::Completed);
    let mut expected = BTreeMap::new();
    expected.insert("approval".into(), Value::String("ok".into()));
    expected.insert("result".into(), Value::Number(42.0));
    assert_eq!(engine.continuation().result, Some(Value::Object(expected)));
}

#[test]
fn sdk_first_example_json_round_trips() {
    let artifact = Artifact::sdk_first_example("sdk-first");
    let text = encode_artifact(&artifact).unwrap();
    let decoded = tcc_ir::decode_artifact(&text).unwrap();
    assert_eq!(decoded, artifact);
}
