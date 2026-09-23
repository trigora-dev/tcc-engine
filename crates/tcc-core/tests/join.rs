use std::collections::BTreeMap;

use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::LocalId;
use tcc_ir::{
    validate, Artifact, ConstValue, EngineCaps, EngineFeature, Envelope, FuncId, Function,
    Instruction, Pc, Program, ENGINE_FORMAT_VERSION, MAX_JOIN_BRANCHES,
};
use tcc_state::{
    decode_continuation, encode_continuation, BranchPhase, ContinuationStatus, JoinKind,
    JoinStatus, Value,
};

fn artifact(instructions: Vec<Instruction>) -> Artifact {
    let spans = vec![None; instructions.len()];
    Artifact {
        envelope: Envelope {
            artifact_hash: "join".into(),
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
        },
        program: Program {
            entry: FuncId(0),
            functions: vec![Function {
                id: FuncId(0),
                name: "run".into(),
                param_count: 0,
                local_count: 2,
                param_defaults: Vec::new(),
                instructions,
                spans,
            }],
        },
    }
}

fn load(text: &str) -> Instruction {
    Instruction::LoadConst {
        value: ConstValue::String(text.into()),
    }
}

fn ack(engine: &mut Engine) {
    match engine.run_until_host(64) {
        EngineOutcome::Host(request) => match request {
            HostRequest::PersistEffect { .. }
            | HostRequest::RegisterWait { .. }
            | HostRequest::RegisterTimer { .. }
            | HostRequest::CreateChild { .. } => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            HostRequest::PersistCheckpoint { revision, .. } => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            HostRequest::RunEffect { key, .. } => {
                let value = match key.as_str() {
                    "a" | "fast" => Value::Number(1.0),
                    "b" | "slow" => Value::Number(2.0),
                    "c" | "third" => Value::Number(3.0),
                    "bad" => {
                        engine
                            .apply_host_response(HostResponse::EffectFailed {
                                message: "nope".into(),
                            })
                            .unwrap();
                        return;
                    }
                    _ => Value::Number(1.0),
                };
                engine
                    .apply_host_response(HostResponse::EffectResult { value })
                    .unwrap();
            }
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("expected host, got {other:?}"),
    }
}

fn drive_to_result(engine: &mut Engine, events: &mut Vec<(String, Value)>) -> Value {
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Completed { result } => return result,
            EngineOutcome::Failed { message } => panic!("failed: {message}"),
            EngineOutcome::Suspended => {
                let branch = engine
                    .continuation()
                    .join
                    .as_ref()
                    .and_then(|join| {
                        join.branches
                            .iter()
                            .find(|branch| branch.phase == BranchPhase::Registered)
                    })
                    .map(|branch| branch.branch_id.clone())
                    .expect("registered branch");
                let value = events
                    .iter()
                    .position(|(id, _)| id == &branch)
                    .map(|index| events.remove(index).1)
                    .unwrap_or(Value::String("ok".into()));
                engine
                    .apply_host_response(HostResponse::EventPayload {
                        value,
                        branch: Some(branch),
                    })
                    .unwrap();
            }
            EngineOutcome::Host(_) => ack(engine),
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn joined_effects_carry_branch_input_on_run_effect() {
    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(11),
        },
        Instruction::NewObject,
        Instruction::LoadConst {
            value: ConstValue::String("left".into()),
        },
        Instruction::SetProp { key: "side".into() },
        load("a"),
        Instruction::Effect { has_input: true },
        Instruction::NewObject,
        Instruction::LoadConst {
            value: ConstValue::String("right".into()),
        },
        Instruction::SetProp { key: "side".into() },
        load("b"),
        Instruction::Effect { has_input: true },
        Instruction::JoinAll,
        Instruction::Return,
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    let mut inputs = Vec::new();
    let result = loop {
        match engine.run_until_host(64) {
            EngineOutcome::Completed { result } => break result,
            EngineOutcome::Host(HostRequest::RunEffect { key, input, .. }) => {
                inputs.push((key, input));
                engine
                    .apply_host_response(HostResponse::EffectResult {
                        value: Value::Number(1.0),
                    })
                    .unwrap();
            }
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(
        inputs,
        vec![
            (
                "a".into(),
                Value::Object(BTreeMap::from([(
                    "side".into(),
                    Value::String("left".into())
                )]))
            ),
            (
                "b".into(),
                Value::Object(BTreeMap::from([(
                    "side".into(),
                    Value::String("right".into())
                )]))
            ),
        ]
    );
    assert_eq!(
        result,
        Value::Array(vec![Value::Number(1.0), Value::Number(1.0)])
    );
}

#[test]
fn effect_without_input_pops_only_the_key() {
    let instructions = vec![
        Instruction::LoadConst {
            value: ConstValue::Number(7.0),
        },
        load("generate"),
        Instruction::Effect { has_input: false },
        Instruction::Return,
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    match engine.run_until_host(64) {
        EngineOutcome::Host(HostRequest::RunEffect { key, input, .. }) => {
            assert_eq!(key, "generate");
            assert_eq!(input, Value::Object(BTreeMap::new()));
            assert_eq!(
                engine.continuation().stack,
                vec![Value::Number(7.0)],
                "a false has_input flag leaves the value under the key on the stack"
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn promise_all_effects_return_input_order() {
    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        load("a"),
        Instruction::Effect { has_input: false },
        load("b"),
        Instruction::Effect { has_input: false },
        Instruction::JoinAll,
        Instruction::Return,
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    let mut events = Vec::new();
    let result = drive_to_result(&mut engine, &mut events);
    assert_eq!(
        result,
        Value::Array(vec![Value::Number(1.0), Value::Number(2.0)])
    );
}

#[test]
fn reverse_event_delivery_keeps_input_order() {
    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        load("left"),
        Instruction::WaitForEvent,
        load("right"),
        Instruction::WaitForEvent,
        Instruction::JoinAll,
        Instruction::Return,
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Suspended => break,
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("setup {other:?}"),
        }
    }
    let join = engine.continuation().join.clone().unwrap();
    assert_eq!(join.state, JoinStatus::Active);
    let first = join.branches[0].branch_id.clone();
    let second = join.branches[1].branch_id.clone();
    engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("second".into()),
            branch: Some(second),
        })
        .unwrap();
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Suspended => break,
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("after second {other:?}"),
        }
    }
    assert_eq!(
        engine.continuation().join.as_ref().unwrap().branches[1].phase,
        BranchPhase::Completed
    );
    engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("first".into()),
            branch: Some(first),
        })
        .unwrap();
    let mut events = Vec::new();
    let result = drive_to_result(&mut engine, &mut events);
    assert_eq!(
        result,
        Value::Array(vec![
            Value::String("first".into()),
            Value::String("second".into())
        ])
    );
}

#[test]
fn join_delivery_without_branch_is_rejected() {
    let instructions = vec![
        Instruction::Fork {
            count: 1,
            join_pc: Pc(3),
        },
        load("ready"),
        Instruction::WaitForEvent,
        Instruction::JoinAll,
        Instruction::Return,
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Suspended => break,
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("setup {other:?}"),
        }
    }
    let err = engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("ok".into()),
            branch: None,
        })
        .unwrap_err();
    assert!(err.to_string().contains("branch"));
}

#[test]
fn artifact_validation_rejects_more_than_32_branches() {
    let mut built = artifact(vec![
        Instruction::Fork {
            count: (MAX_JOIN_BRANCHES as u32) + 1,
            join_pc: Pc(1),
        },
        Instruction::JoinAll,
        Instruction::Return,
    ]);
    let err = validate(&built, &EngineCaps::current()).unwrap_err();
    assert!(err.to_string().contains("join branches"));
    built.program.functions[0].instructions[0] = Instruction::Fork {
        count: 2,
        join_pc: Pc(1),
    };
    built.envelope.required_engine_features.clear();
    let err = validate(&built, &EngineCaps::current()).unwrap_err();
    assert!(err.to_string().contains("durable.concurrent_group"));
    let _ = EngineFeature::DURABLE_CONCURRENT_GROUP;
}

#[test]
fn failure_is_persisted_before_the_throw_and_stale_delivery_is_ignored() {
    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        load("bad"),
        Instruction::Effect { has_input: false },
        load("later"),
        Instruction::WaitForEvent,
        Instruction::JoinAll,
        Instruction::Return,
    ];
    let program = artifact(instructions);
    let mut engine = Engine::start(program.clone(), "exec", &EngineCaps::current()).unwrap();
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { .. }) => {
                let join = engine.continuation().join.clone().unwrap();
                if join.state == JoinStatus::Failed {
                    assert_eq!(join.failure_branch, Some(0));
                    assert_eq!(join.branches[1].phase, BranchPhase::Detached);
                    let saved = engine.continuation().clone();
                    let sibling = saved.join.as_ref().unwrap().branches[1].branch_id.clone();
                    ack(&mut engine);
                    let mut resumed =
                        Engine::resume(program.clone(), saved, &EngineCaps::current()).unwrap();
                    resumed
                        .apply_host_response(HostResponse::EventPayload {
                            value: Value::String("stale".into()),
                            branch: Some(sibling),
                        })
                        .unwrap();
                    assert_eq!(
                        resumed.continuation().join.as_ref().unwrap().state,
                        JoinStatus::Failed
                    );
                    let outcome = resumed.run_until_host(64);
                    assert!(
                        matches!(
                            outcome,
                            EngineOutcome::Host(HostRequest::PersistCheckpoint { .. })
                        ),
                        "the throw is checkpointed once, got {outcome:?}"
                    );
                    assert!(resumed.continuation().join.is_none());
                    assert_eq!(resumed.continuation().status, ContinuationStatus::Failed);
                    ack(&mut resumed);
                    let outcome = resumed.run_until_host(64);
                    assert!(
                        matches!(outcome, EngineOutcome::Failed { .. }),
                        "resume throws once after the failure checkpoint, got {outcome:?}"
                    );
                    return;
                }
                ack(&mut engine);
            }
            EngineOutcome::Host(_) => ack(&mut engine),
            EngineOutcome::Failed { message } => {
                panic!("threw before the failed join was checkpointed: {message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn reentry_does_not_alias_branch_ids() {
    let instructions = vec![
        Instruction::Fork {
            count: 1,
            join_pc: Pc(3),
        },
        load("a"),
        Instruction::Effect { has_input: false },
        Instruction::JoinAll,
        Instruction::Jump { target: Pc(0) },
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    let mut seen = Vec::new();
    for _ in 0..40 {
        match engine.run_until_host(32) {
            EngineOutcome::Host(HostRequest::RunEffect { .. }) => {
                if let Some(join) = engine.continuation().join.clone() {
                    seen.push(join.branches[0].branch_id.clone());
                }
                engine
                    .apply_host_response(HostResponse::EffectResult {
                        value: Value::Number(1.0),
                    })
                    .unwrap();
            }
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("{other:?}"),
        }
        if seen.len() == 2 {
            break;
        }
    }
    assert_eq!(seen.len(), 2);
    assert_ne!(seen[0], seen[1]);
    assert!(seen[0].ends_with(":0"));
    assert!(seen[1].contains(":1:0") || seen[1].ends_with(":0"));
    let mut ids = BTreeMap::new();
    for id in &seen {
        *ids.entry(id.clone()).or_insert(0) += 1;
    }
    assert!(ids.values().all(|count| *count == 1));
}

fn race_effects(keys: &[&str]) -> Vec<Instruction> {
    let mut instructions = vec![Instruction::Fork {
        count: keys.len() as u32,
        join_pc: Pc(0),
    }];
    for key in keys {
        instructions.push(load(key));
        instructions.push(Instruction::Effect { has_input: false });
    }
    let join_pc = Pc(instructions.len() as u32);
    instructions.push(Instruction::JoinAny);
    instructions.push(Instruction::Return);
    if let Instruction::Fork { join_pc: slot, .. } = &mut instructions[0] {
        *slot = join_pc;
    }
    instructions
}

fn drive_until_suspended(engine: &mut Engine) {
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Suspended => return,
            EngineOutcome::Host(_) => ack(engine),
            other => panic!("setup {other:?}"),
        }
    }
}

#[test]
fn race_runs_every_effect_then_lowest_index_wins() {
    // Several effects can finish during registration. The host does not deliver
    // two wakes in one transition; the settle checkpoint applies the lowest-index rule.
    let mut engine = Engine::start(
        artifact(race_effects(&["fast", "slow", "third"])),
        "exec",
        &EngineCaps::current(),
    )
    .unwrap();
    let mut seen = Vec::new();
    let result = loop {
        match engine.run_until_host(64) {
            EngineOutcome::Completed { result } => break result,
            EngineOutcome::Host(HostRequest::RunEffect { key, .. }) => {
                seen.push(key);
                ack(&mut engine);
            }
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(seen, vec!["fast", "slow", "third"]);
    assert_eq!(result, Value::Number(1.0));
}

#[test]
fn race_settle_checkpoint_stores_the_winning_value() {
    let program = artifact(race_effects(&["a", "b"]));
    let mut engine = Engine::start(program.clone(), "exec", &EngineCaps::current()).unwrap();
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { .. }) => {
                let join = engine.continuation().join.clone().unwrap();
                if join.state == JoinStatus::Succeeded {
                    assert_eq!(join.kind, JoinKind::Any);
                    assert_eq!(join.winner_branch, Some(0));
                    assert_eq!(join.failure_branch, None);
                    assert_eq!(join.branches[0].result, Some(Value::Number(1.0)));
                    assert_eq!(join.branches[1].phase, BranchPhase::Completed);
                    let saved = engine.continuation().clone();
                    ack(&mut engine);
                    let mut events = Vec::new();
                    assert_eq!(
                        drive_to_result(&mut engine, &mut events),
                        Value::Number(1.0)
                    );
                    let mut resumed =
                        Engine::resume(program, saved, &EngineCaps::current()).unwrap();
                    resumed
                        .apply_host_response(HostResponse::EventPayload {
                            value: Value::String("stale".into()),
                            branch: Some(
                                resumed.continuation().join.as_ref().unwrap().branches[1]
                                    .branch_id
                                    .clone(),
                            ),
                        })
                        .unwrap();
                    assert_eq!(
                        resumed.continuation().join.as_ref().unwrap().winner_branch,
                        Some(0)
                    );
                    assert_eq!(
                        drive_to_result(&mut resumed, &mut events),
                        Value::Number(1.0)
                    );
                    return;
                }
                ack(&mut engine);
            }
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn race_timer_and_wait_either_can_win_after_both_register() {
    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        load("ready"),
        Instruction::WaitForEvent,
        Instruction::LoadConst {
            value: ConstValue::Number(5.0),
        },
        Instruction::Sleep,
        Instruction::JoinAny,
        Instruction::Return,
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    drive_until_suspended(&mut engine);
    let join = engine.continuation().join.clone().unwrap();
    assert!(join.winner_branch.is_none());
    assert_eq!(join.branches[0].phase, BranchPhase::Registered);
    assert_eq!(join.branches[1].phase, BranchPhase::Registered);
    let timer = join.branches[1].branch_id.clone();
    engine
        .apply_host_response(HostResponse::TimerFired {
            branch: Some(timer),
        })
        .unwrap();
    let mut events = Vec::new();
    assert_eq!(drive_to_result(&mut engine, &mut events), Value::Undefined);

    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        load("ready"),
        Instruction::WaitForEvent,
        Instruction::LoadConst {
            value: ConstValue::Number(5.0),
        },
        Instruction::Sleep,
        Instruction::JoinAny,
        Instruction::Return,
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    drive_until_suspended(&mut engine);
    let wait = engine.continuation().join.as_ref().unwrap().branches[0]
        .branch_id
        .clone();
    engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("go".into()),
            branch: Some(wait),
        })
        .unwrap();
    assert_eq!(
        drive_to_result(&mut engine, &mut events),
        Value::String("go".into())
    );
    assert!(engine.continuation().join.is_none());
}

#[test]
fn race_reverse_delivery_lets_the_higher_index_win() {
    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        load("left"),
        Instruction::WaitForEvent,
        load("right"),
        Instruction::WaitForEvent,
        Instruction::JoinAny,
        Instruction::Return,
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    drive_until_suspended(&mut engine);
    let right = engine.continuation().join.as_ref().unwrap().branches[1]
        .branch_id
        .clone();
    engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("second".into()),
            branch: Some(right),
        })
        .unwrap();
    let mut events = Vec::new();
    assert_eq!(
        drive_to_result(&mut engine, &mut events),
        Value::String("second".into())
    );
}

#[test]
fn race_same_name_wait_ignores_a_later_delivery() {
    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        load("ready"),
        Instruction::WaitForEvent,
        load("ready"),
        Instruction::WaitForEvent,
        Instruction::JoinAny,
        Instruction::Return,
    ];
    let program = artifact(instructions);
    let mut engine = Engine::start(program.clone(), "exec", &EngineCaps::current()).unwrap();
    drive_until_suspended(&mut engine);
    let join = engine.continuation().join.clone().unwrap();
    let first = join.branches[0].branch_id.clone();
    let second = join.branches[1].branch_id.clone();
    engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("first".into()),
            branch: Some(first),
        })
        .unwrap();
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { .. }) => {
                let join = engine.continuation().join.clone().unwrap();
                if join.state == JoinStatus::Succeeded {
                    assert_eq!(join.winner_branch, Some(0));
                    assert_eq!(join.branches[1].phase, BranchPhase::Detached);
                    let saved = engine.continuation().clone();
                    let mut resumed =
                        Engine::resume(program, saved, &EngineCaps::current()).unwrap();
                    resumed
                        .apply_host_response(HostResponse::EventPayload {
                            value: Value::String("later".into()),
                            branch: Some(second),
                        })
                        .unwrap();
                    let mut events = Vec::new();
                    assert_eq!(
                        drive_to_result(&mut resumed, &mut events),
                        Value::String("first".into())
                    );
                    return;
                }
                ack(&mut engine);
            }
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn race_effect_failure_wins_and_catch_runs_once_across_resume() {
    let instructions = vec![
        Instruction::PushTry {
            catch: Pc(9),
            finally: None,
        },
        Instruction::Fork {
            count: 2,
            join_pc: Pc(6),
        },
        load("bad"),
        Instruction::Effect { has_input: false },
        load("slow"),
        Instruction::Effect { has_input: false },
        Instruction::JoinAny,
        Instruction::PopTry,
        Instruction::Jump { target: Pc(12) },
        Instruction::StoreLocal { local: LocalId(0) },
        load("caught"),
        Instruction::Return,
        load("fell"),
        Instruction::Return,
    ];
    let program = artifact(instructions);
    let mut engine = Engine::start(program.clone(), "exec", &EngineCaps::current()).unwrap();
    let mut saw_slow = false;
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Host(HostRequest::RunEffect { key, .. }) => {
                if key == "slow" {
                    saw_slow = true;
                }
                ack(&mut engine);
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { .. }) => {
                let join = engine.continuation().join.clone();
                if join
                    .as_ref()
                    .is_some_and(|join| join.state == JoinStatus::Failed)
                {
                    let join = join.unwrap();
                    assert_eq!(join.winner_branch, Some(0));
                    assert_eq!(join.failure_branch, Some(0));
                    assert!(saw_slow);
                    let saved = engine.continuation().clone();
                    let mut resumed =
                        Engine::resume(program.clone(), saved, &EngineCaps::current()).unwrap();
                    let mut events = Vec::new();
                    assert_eq!(
                        drive_to_result(&mut resumed, &mut events),
                        Value::String("caught".into())
                    );
                    assert!(resumed.continuation().join.is_none());
                    assert_eq!(resumed.continuation().status, ContinuationStatus::Completed);
                    assert_eq!(
                        resumed.continuation().result,
                        Some(Value::String("caught".into()))
                    );
                    return;
                }
                ack(&mut engine);
            }
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn race_delivery_without_branch_is_rejected() {
    let instructions = vec![
        Instruction::Fork {
            count: 1,
            join_pc: Pc(3),
        },
        load("ready"),
        Instruction::WaitForEvent,
        Instruction::JoinAny,
        Instruction::Return,
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    drive_until_suspended(&mut engine);
    let err = engine
        .apply_host_response(HostResponse::EventPayload {
            value: Value::String("ok".into()),
            branch: None,
        })
        .unwrap_err();
    assert!(err.to_string().contains("branch"));
}

#[test]
fn race_reentry_does_not_alias_branch_ids() {
    let instructions = vec![
        Instruction::Fork {
            count: 1,
            join_pc: Pc(3),
        },
        load("a"),
        Instruction::Effect { has_input: false },
        Instruction::JoinAny,
        Instruction::Jump { target: Pc(0) },
    ];
    let mut engine = Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap();
    let mut seen = Vec::new();
    for _ in 0..80 {
        match engine.run_until_host(32) {
            EngineOutcome::Host(HostRequest::RunEffect { .. }) => {
                if let Some(join) = engine.continuation().join.clone() {
                    if join.branches[0].phase == BranchPhase::Planned {
                        seen.push(join.branches[0].branch_id.clone());
                    }
                }
                ack(&mut engine);
            }
            EngineOutcome::Host(_) => ack(&mut engine),
            other => panic!("{other:?}"),
        }
        if seen.len() == 2 {
            break;
        }
    }
    assert_eq!(seen.len(), 2);
    assert_ne!(seen[0], seen[1]);
}

#[test]
fn missing_join_kind_fails_decode() {
    let mut engine = Engine::start(
        artifact(race_effects(&["a"])),
        "exec",
        &EngineCaps::current(),
    )
    .unwrap();
    loop {
        match engine.run_until_host(16) {
            EngineOutcome::Host(_) => {
                if engine.continuation().join.is_some() {
                    break;
                }
                ack(&mut engine);
            }
            other => panic!("{other:?}"),
        }
    }
    let text = String::from_utf8(encode_continuation(engine.continuation()).unwrap()).unwrap();
    assert!(text.contains("\"kind\":\"any\""));
    let stripped = text.replacen("\"kind\":\"any\",", "", 1);
    assert!(decode_continuation(stripped.as_bytes()).is_err());
}
