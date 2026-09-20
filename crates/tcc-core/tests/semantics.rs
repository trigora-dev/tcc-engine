use std::collections::BTreeMap;

use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{
    Artifact, ConstValue, EngineCaps, Envelope, FuncId, Function, Instruction, LocalId, Pc,
    Program, ENGINE_FORMAT_VERSION,
};
use tcc_state::{ContinuationStatus, Value};

fn drive(engine: &mut Engine, effects: &BTreeMap<&str, Value>, event: Option<Value>) -> Value {
    let mut event = event;
    loop {
        match engine.run_until_host(256) {
            EngineOutcome::Host(HostRequest::RunEffect { key, .. }) => {
                let value = effects
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or_else(|| panic!("missing effect {key}"));
                engine
                    .apply_host_response(HostResponse::EffectResult { value })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistEffect { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::RegisterWait { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::RegisterTimer { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::CreateChild { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Suspended => match &engine.continuation().pending {
                Some(tcc_state::PendingOp::Wait {
                    kind: tcc_state::WaitKind::Timer { .. },
                }) => engine
                    .apply_host_response(HostResponse::TimerFired)
                    .unwrap(),
                Some(tcc_state::PendingOp::Wait {
                    kind: tcc_state::WaitKind::Child { .. },
                }) => engine
                    .apply_host_response(HostResponse::ChildResult {
                        value: Value::Number(7.0),
                    })
                    .unwrap(),
                _ => {
                    let value = event.take().expect("event payload");
                    engine
                        .apply_host_response(HostResponse::EventPayload { value })
                        .unwrap();
                }
            },
            EngineOutcome::Completed { result } => return result,
            EngineOutcome::Cancelled => panic!("cancelled"),
            EngineOutcome::Failed { message } => panic!("failed: {message}"),
            EngineOutcome::BudgetExhausted => panic!("budget"),
            EngineOutcome::Host(other) => panic!("unexpected host request {other:?}"),
        }
    }
}

fn envelope() -> Envelope {
    Envelope {
        artifact_hash: "semantics".into(),
        frontend_id: "typescript".into(),
        frontend_version: "0.0.0".into(),
        language_semantics_version: "ts.subset.v1".into(),
        engine_format_version: ENGINE_FORMAT_VERSION,
        required_engine_features: tcc_ir::FeatureSet::current_engine(),
        required_host_capabilities: tcc_ir::HostCapability::known()
            .iter()
            .map(|id| tcc_ir::HostCapability((*id).to_string()))
            .collect(),
        runtime_modules: Vec::new(),
    }
}

fn artifact(local_count: u32, instructions: Vec<Instruction>) -> Artifact {
    let spans = vec![None; instructions.len()];
    Artifact {
        envelope: envelope(),
        program: Program {
            entry: FuncId(0),
            functions: vec![Function {
                id: FuncId(0),
                name: "run".into(),
                param_count: 0,
                local_count,
                instructions,
                spans,
            }],
        },
    }
}

#[test]
fn if_else_skips_untaken_effect() {
    let artifact = artifact(
        2,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("flag".into()),
            },
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::JumpIfFalse { target: Pc(10) },
            Instruction::LoadConst {
                value: ConstValue::String("taken".into()),
            },
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(1) },
            Instruction::LoadLocal { local: LocalId(1) },
            Instruction::Return,
            Instruction::LoadConst {
                value: ConstValue::String("skipped".into()),
            },
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(1) },
            Instruction::LoadLocal { local: LocalId(1) },
            Instruction::Return,
        ],
    );
    let mut engine = Engine::start(artifact, "exec", &EngineCaps::current()).unwrap();
    let mut requested = Vec::new();
    loop {
        match engine.run_until_host(256) {
            EngineOutcome::Host(HostRequest::RunEffect { key, .. }) => {
                requested.push(key.clone());
                let value = if key == "flag" {
                    Value::Number(1.0)
                } else {
                    Value::Number(42.0)
                };
                engine
                    .apply_host_response(HostResponse::EffectResult { value })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistEffect { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Completed { result } => {
                assert_eq!(result, Value::Number(42.0));
                break;
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(requested, vec!["flag".to_string(), "taken".to_string()]);
}

#[test]
fn comparisons_and_arrays() {
    let artifact = artifact(
        1,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("n".into()),
            },
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::LoadConst {
                value: ConstValue::Number(3.0),
            },
            Instruction::StrictEq,
            Instruction::JumpIfFalse { target: Pc(14) },
            Instruction::NewArray,
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::ArrayPush,
            Instruction::LoadConst {
                value: ConstValue::Number(1.0),
            },
            Instruction::ArrayPush,
            Instruction::Return,
            Instruction::Nop,
            Instruction::LoadConst {
                value: ConstValue::Bool(false),
            },
            Instruction::Return,
        ],
    );
    let mut engine = Engine::start(artifact, "exec", &EngineCaps::current()).unwrap();
    let mut effects = BTreeMap::new();
    effects.insert("n", Value::Number(3.0));
    let result = drive(&mut engine, &effects, None);
    assert_eq!(
        result,
        Value::Array(vec![Value::Number(3.0), Value::Number(1.0)])
    );
}

#[test]
fn try_catch_handles_throw() {
    let artifact = artifact(
        1,
        vec![
            Instruction::PushTry {
                catch: Pc(5),
                finally: None,
            },
            Instruction::LoadConst {
                value: ConstValue::String("boom".into()),
            },
            Instruction::Throw,
            Instruction::PopTry,
            Instruction::Jump { target: Pc(8) },
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Return,
            Instruction::LoadConst {
                value: ConstValue::Null,
            },
            Instruction::Return,
        ],
    );
    let mut engine = Engine::start(artifact, "exec", &EngineCaps::current()).unwrap();
    let result = drive(&mut engine, &BTreeMap::new(), None);
    assert_eq!(result, Value::String("boom".into()));
}

#[test]
fn sleep_and_invoke_wake() {
    let sleep = artifact(
        0,
        vec![
            Instruction::LoadConst {
                value: ConstValue::Number(0.0),
            },
            Instruction::Sleep,
            Instruction::Pop,
            Instruction::LoadConst {
                value: ConstValue::Number(1.0),
            },
            Instruction::Return,
        ],
    );
    let mut engine = Engine::start(sleep, "exec", &EngineCaps::current()).unwrap();
    assert_eq!(
        drive(&mut engine, &BTreeMap::new(), None),
        Value::Number(1.0)
    );

    let invoke = artifact(
        1,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("child".into()),
            },
            Instruction::Invoke,
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Return,
        ],
    );
    let mut engine = Engine::start(invoke, "exec", &EngineCaps::current()).unwrap();
    assert_eq!(
        drive(&mut engine, &BTreeMap::new(), None),
        Value::Number(7.0)
    );
}

#[test]
fn cancel_while_suspended() {
    let artifact = artifact(
        0,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("never".into()),
            },
            Instruction::WaitForEvent,
            Instruction::Return,
        ],
    );
    let mut engine = Engine::start(artifact, "exec", &EngineCaps::current()).unwrap();
    loop {
        match engine.run_until_host(32) {
            EngineOutcome::Host(HostRequest::RegisterWait { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Suspended => {
                engine.apply_host_response(HostResponse::Cancel).unwrap();
            }
            EngineOutcome::Cancelled => {
                assert_eq!(engine.continuation().status, ContinuationStatus::Cancelled);
                return;
            }
            other => panic!("{other:?}"),
        }
    }
}
