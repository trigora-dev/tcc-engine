use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{
    Artifact, ConstValue, EngineCaps, EngineFeature, Envelope, FuncId, Function, HostCapability,
    Instruction, LocalId, Program, ENGINE_FORMAT_VERSION, FRONTEND_TYPESCRIPT,
    LANGUAGE_SEMANTICS_TS,
};
use tcc_state::PersistKind;
use tcc_state::{ContinuationStatus, PendingOp, Value, WaitKind};

fn first_example() -> Artifact {
    Artifact {
        envelope: Envelope {
            artifact_hash: "first-example".into(),
            frontend_id: FRONTEND_TYPESCRIPT.into(),
            frontend_version: "0.0.0".into(),
            language_semantics_version: LANGUAGE_SEMANTICS_TS.into(),
            engine_format_version: ENGINE_FORMAT_VERSION,
            required_engine_features: vec![
                EngineFeature(EngineFeature::TS_CONTROL_FLOW.into()),
                EngineFeature(EngineFeature::DURABLE_EFFECT.into()),
                EngineFeature(EngineFeature::DURABLE_WAIT_FOR_EVENT.into()),
            ],
            required_host_capabilities: vec![
                HostCapability(HostCapability::PERSIST_CHECKPOINT.into()),
                HostCapability(HostCapability::EFFECT.into()),
                HostCapability(HostCapability::EVENT.into()),
            ],
            runtime_modules: Vec::new(),
        },
        program: Program {
            entry: FuncId(0),
            functions: vec![Function {
                id: FuncId(0),
                name: "main".into(),
                param_count: 0,
                local_count: 1,
                param_defaults: Vec::new(),
                instructions: vec![
                    Instruction::LoadConst {
                        value: ConstValue::String("charge".into()),
                    },
                    Instruction::Effect { has_input: false },
                    Instruction::StoreLocal { local: LocalId(0) },
                    Instruction::LoadConst {
                        value: ConstValue::String("approved".into()),
                    },
                    Instruction::WaitForEvent,
                    Instruction::Pop,
                    Instruction::LoadLocal { local: LocalId(0) },
                    Instruction::Return,
                ],
                spans: vec![None; 8],
            }],
        },
    }
}

fn start() -> Engine {
    Engine::start(first_example(), "first", &EngineCaps::current()).unwrap()
}

fn expect_host(outcome: EngineOutcome) -> HostRequest {
    match outcome {
        EngineOutcome::Host(request) => request,
        other => panic!("expected host request, got {other:?}"),
    }
}

#[test]
fn first_example_runs_effect_wait_and_completes() {
    let mut engine = start();

    let request = expect_host(engine.run_until_host(32));
    assert_eq!(
        request,
        HostRequest::RunEffect {
            key: "charge".into(),
            idempotency_key: "first:charge".into(),
            input: Value::Object(std::collections::BTreeMap::new()),
        }
    );
    assert_eq!(engine.continuation().revision, 0);
    engine
        .apply_host_response(HostResponse::EffectResult {
            value: Value::Number(42.0),
        })
        .unwrap();
    assert_eq!(engine.continuation().frames[0].pc, 2);
    assert_eq!(engine.continuation().stack, vec![Value::Number(42.0)]);
    assert_eq!(engine.continuation().pending, None);

    let request = expect_host(engine.run_until_host(32));
    match request {
        HostRequest::PersistEffect { record } => {
            assert_eq!(record.key, "charge");
            assert_eq!(record.idempotency_key, "first:charge");
            assert_eq!(record.result, Some(Value::Number(42.0)));
        }
        other => panic!("expected persist_effect, got {other:?}"),
    }
    engine.apply_host_response(HostResponse::Ack).unwrap();

    let request = expect_host(engine.run_until_host(32));
    assert!(matches!(
        request,
        HostRequest::PersistCheckpoint {
            revision: 1,
            kind: PersistKind::Snapshot,
            materialize: true,
            ..
        }
    ));
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision: 1 })
        .unwrap();
    assert_eq!(engine.continuation().revision, 1);
    assert_eq!(engine.continuation().status, ContinuationStatus::Runnable);

    let request = expect_host(engine.run_until_host(32));
    match request {
        HostRequest::RegisterWait { wait } => {
            assert_eq!(wait.wait_id, "first:approved::4");
            assert_eq!(wait.event_name, "approved");
            assert_eq!(wait.correlation_key, None);
        }
        other => panic!("expected register_wait, got {other:?}"),
    }
    engine.apply_host_response(HostResponse::Ack).unwrap();
    assert_eq!(engine.continuation().status, ContinuationStatus::Suspended);
    match &engine.continuation().pending {
        Some(PendingOp::Wait {
            kind:
                WaitKind::Event {
                    wait_id,
                    event_name,
                    ..
                },
        }) => {
            assert_eq!(wait_id, "first:approved::4");
            assert_eq!(event_name, "approved");
        }
        other => panic!("expected event wait, got {other:?}"),
    }

    let request = expect_host(engine.run_until_host(32));
    assert!(matches!(
        request,
        HostRequest::PersistCheckpoint {
            revision: 2,
            kind: PersistKind::Delta,
            materialize: false,
            base_revision: 1,
            ..
        }
    ));
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision: 2 })
        .unwrap();
    assert_eq!(engine.continuation().revision, 2);
    assert_eq!(engine.continuation().status, ContinuationStatus::Suspended);
    assert_eq!(engine.run_until_host(32), EngineOutcome::Suspended);

    engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("ok".into()),
            branch: None,
        })
        .unwrap();
    assert_eq!(engine.continuation().pending, None);
    assert_eq!(
        engine.continuation().stack,
        vec![Value::String("ok".into())]
    );

    let request = expect_host(engine.run_until_host(32));
    assert!(matches!(
        request,
        HostRequest::PersistCheckpoint {
            revision: 3,
            kind: PersistKind::Delta,
            materialize: false,
            base_revision: 2,
            ..
        }
    ));
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision: 3 })
        .unwrap();
    assert_eq!(engine.continuation().revision, 3);
    assert_eq!(engine.continuation().status, ContinuationStatus::Runnable);

    let request = expect_host(engine.run_until_host(32));
    assert!(matches!(
        request,
        HostRequest::PersistCheckpoint {
            revision: 4,
            kind: PersistKind::Snapshot,
            materialize: true,
            ..
        }
    ));
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision: 4 })
        .unwrap();
    assert_eq!(engine.continuation().status, ContinuationStatus::Completed);
    assert_eq!(engine.continuation().result, Some(Value::Number(42.0)));
    assert_eq!(engine.continuation().revision, 4);
}

#[test]
fn first_example_budget_zero_does_not_commit() {
    let mut engine = start();
    assert_eq!(engine.run_until_host(0), EngineOutcome::BudgetExhausted);
    assert_eq!(engine.continuation().frames[0].pc, 0);
    assert_eq!(engine.continuation().revision, 0);
    assert_eq!(engine.continuation().status, ContinuationStatus::Running);
}
