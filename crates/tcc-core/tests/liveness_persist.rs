use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{
    Artifact, ConstValue, EngineCaps, EngineFeature, Envelope, FuncId, Function, HostCapability,
    Instruction, LocalId, Program, ENGINE_FORMAT_VERSION, FRONTEND_TYPESCRIPT,
    LANGUAGE_SEMANTICS_TS,
};
use tcc_state::{LocalPatch, PersistKind, Value};

fn expect_host(outcome: EngineOutcome) -> HostRequest {
    match outcome {
        EngineOutcome::Host(request) => request,
        other => panic!("expected host request, got {other:?}"),
    }
}

fn wait_artifact() -> Artifact {
    Artifact {
        envelope: Envelope {
            artifact_hash: "live-kill".into(),
            frontend_id: FRONTEND_TYPESCRIPT.to_string(),
            frontend_version: "0.0.0".to_string(),
            language_semantics_version: LANGUAGE_SEMANTICS_TS.to_string(),
            engine_format_version: ENGINE_FORMAT_VERSION,
            required_engine_features: vec![
                EngineFeature(EngineFeature::TS_CONTROL_FLOW.to_string()),
                EngineFeature(EngineFeature::DURABLE_WAIT_FOR_EVENT.to_string()),
            ],
            required_host_capabilities: vec![
                HostCapability(HostCapability::PERSIST_CHECKPOINT.to_string()),
                HostCapability(HostCapability::EVENT.to_string()),
            ],
            runtime_modules: Vec::new(),
        },
        program: Program {
            entry: FuncId(0),
            functions: vec![Function {
                id: FuncId(0),
                name: "run".into(),
                param_count: 0,
                local_count: 3,
                instructions: vec![
                    Instruction::LoadConst {
                        value: ConstValue::String("keep".into()),
                    },
                    Instruction::StoreLocal { local: LocalId(0) },
                    Instruction::LoadConst {
                        value: ConstValue::String("dead-payload".into()),
                    },
                    Instruction::StoreLocal { local: LocalId(1) },
                    Instruction::LoadConst {
                        value: ConstValue::String("first".into()),
                    },
                    Instruction::WaitForEvent,
                    Instruction::StoreLocal { local: LocalId(2) },
                    Instruction::LoadLocal { local: LocalId(1) },
                    Instruction::Pop,
                    Instruction::LoadConst {
                        value: ConstValue::String("second".into()),
                    },
                    Instruction::WaitForEvent,
                    Instruction::Nop,
                    Instruction::LoadLocal { local: LocalId(0) },
                    Instruction::Return,
                ],
                spans: vec![None; 14],
            }],
        },
    }
}

fn confirm_persist(engine: &mut Engine, revision: u64) -> HostRequest {
    let request = expect_host(engine.run_until_host(64));
    match &request {
        HostRequest::PersistCheckpoint {
            revision: found, ..
        } => assert_eq!(*found, revision),
        other => panic!("expected persist_checkpoint, got {other:?}"),
    }
    engine
        .apply_host_response(HostResponse::PersistConfirmed { revision })
        .unwrap();
    request
}

fn ack_wait(engine: &mut Engine, event_name: &str) {
    match expect_host(engine.run_until_host(64)) {
        HostRequest::RegisterWait { wait } => {
            assert_eq!(wait.event_name, event_name);
        }
        other => panic!("expected register_wait, got {other:?}"),
    }
    engine.apply_host_response(HostResponse::Ack).unwrap();
}

#[test]
fn persist_after_last_use_patches_dead_slot_to_undefined() {
    let mut engine = Engine::start(wait_artifact(), "live-exec", &EngineCaps::current()).unwrap();

    ack_wait(&mut engine, "first");
    let first = confirm_persist(&mut engine, 1);
    assert!(matches!(
        first,
        HostRequest::PersistCheckpoint {
            kind: PersistKind::Snapshot,
            ..
        }
    ));
    assert_eq!(
        engine.continuation().frames[0].locals[0],
        Value::String("keep".into())
    );
    assert_eq!(
        engine.continuation().frames[0].locals[1],
        Value::String("dead-payload".into())
    );
    assert_eq!(engine.run_until_host(8), EngineOutcome::Suspended);

    engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("ok".into()),
            branch: None,
        })
        .unwrap();
    confirm_persist(&mut engine, 2);

    ack_wait(&mut engine, "second");
    let second = confirm_persist(&mut engine, 3);
    match second {
        HostRequest::PersistCheckpoint {
            kind: PersistKind::Delta,
            delta: Some(delta),
            ..
        } => {
            let patches: Vec<&LocalPatch> = delta
                .frames
                .iter()
                .flat_map(|frame| frame.locals.iter())
                .collect();
            assert!(
                patches
                    .iter()
                    .any(|patch| patch.slot == 1 && patch.value == Value::Undefined),
                "dead slot must be patched to undefined, not omitted: {delta:?}"
            );
            assert!(
                !patches
                    .iter()
                    .any(|patch| patch.slot == 0 && patch.value == Value::Undefined),
                "live slot must not be killed: {delta:?}"
            );
        }
        other => panic!("expected delta persist, got {other:?}"),
    }
    assert_eq!(
        engine.continuation().frames[0].locals[0],
        Value::String("keep".into())
    );
    assert_eq!(engine.continuation().frames[0].locals[1], Value::Undefined);
}
