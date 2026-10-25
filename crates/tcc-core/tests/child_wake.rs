use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{
    Artifact, ConstValue, EngineCaps, Envelope, FuncId, Function, Instruction, Pc, Program,
    ENGINE_FORMAT_VERSION,
};
use tcc_state::{BranchPhase, ContinuationStatus, JoinStatus, Value};

fn artifact(instructions: Vec<Instruction>) -> Artifact {
    let spans = vec![None; instructions.len()];
    Artifact {
        envelope: Envelope {
            artifact_hash: "child-wake".into(),
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
                local_count: 1,
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

struct StartedChild {
    invoke_id: String,
    child_execution_id: String,
}

fn pump(engine: &mut Engine) -> (EngineOutcome, Vec<StartedChild>) {
    let mut children = Vec::new();
    let outcome = loop {
        match engine.run_until_host(64) {
            EngineOutcome::Host(HostRequest::CreateChild { child }) => {
                children.push(StartedChild {
                    invoke_id: child.invoke_id,
                    child_execution_id: child.child_execution_id,
                });
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistEffect { .. })
            | EngineOutcome::Host(HostRequest::RegisterWait { .. })
            | EngineOutcome::Host(HostRequest::RegisterTimer { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::RunEffect { .. }) => {
                engine
                    .apply_host_response(HostResponse::EffectFailed {
                        message: "nope".into(),
                    })
                    .unwrap();
            }
            other => break other,
        }
    };
    (outcome, children)
}

fn finish(engine: &mut Engine) -> EngineOutcome {
    loop {
        match engine.run_until_host(64) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            other => return other,
        }
    }
}

fn reject(engine: &mut Engine, response: HostResponse, text: &str) {
    let before = engine.continuation().clone();
    let message = engine
        .apply_host_response(response)
        .unwrap_err()
        .to_string();
    assert!(message.contains(text), "{message}");
    assert_eq!(engine.continuation(), &before);
}

fn start(instructions: Vec<Instruction>) -> Engine {
    Engine::start(artifact(instructions), "exec", &EngineCaps::current()).unwrap()
}

#[test]
fn cancel_does_not_apply_to_a_terminal_continuation() {
    let mut completed = start(vec![
        Instruction::LoadConst {
            value: ConstValue::Number(1.0),
        },
        Instruction::Return,
    ]);
    assert_eq!(
        pump(&mut completed).0,
        EngineOutcome::Completed {
            result: Value::Number(1.0)
        }
    );
    reject(
        &mut completed,
        HostResponse::Cancel,
        "terminal continuation",
    );
    assert_eq!(
        completed.continuation().status,
        ContinuationStatus::Completed
    );

    let mut failed = start(vec![
        load("bad"),
        Instruction::Effect { has_input: false },
        Instruction::Return,
    ]);
    assert_eq!(
        pump(&mut failed).0,
        EngineOutcome::Failed {
            message: "nope".into()
        }
    );
    reject(&mut failed, HostResponse::Cancel, "terminal continuation");

    let mut cancelled = start(vec![
        load("never"),
        Instruction::WaitForEvent,
        Instruction::Return,
    ]);
    assert_eq!(pump(&mut cancelled).0, EngineOutcome::Suspended);
    cancelled.apply_host_response(HostResponse::Cancel).unwrap();
    assert_eq!(finish(&mut cancelled), EngineOutcome::Cancelled);
    reject(
        &mut cancelled,
        HostResponse::Cancel,
        "terminal continuation",
    );
    assert_eq!(
        cancelled.continuation().status,
        ContinuationStatus::Cancelled
    );
}

#[test]
fn child_result_must_name_the_waiting_child() {
    let mut engine = start(vec![
        load("child"),
        Instruction::Invoke { arg_count: 0 },
        Instruction::Return,
    ]);
    let (outcome, children) = pump(&mut engine);
    assert_eq!(outcome, EngineOutcome::Suspended);
    let child_execution_id = children[0].child_execution_id.clone();
    reject(
        &mut engine,
        HostResponse::ChildResult {
            value: Value::Number(7.0),
            child_execution_id: "child:other".into(),
            branch: None,
        },
        &child_execution_id,
    );
    engine
        .apply_host_response(HostResponse::ChildResult {
            value: Value::Number(7.0),
            child_execution_id,
            branch: None,
        })
        .unwrap();
    assert_eq!(
        finish(&mut engine),
        EngineOutcome::Completed {
            result: Value::Number(7.0)
        }
    );
}

#[test]
fn child_failure_propagates_only_for_the_waiting_child() {
    let mut engine = start(vec![
        load("child"),
        Instruction::Invoke { arg_count: 0 },
        Instruction::Return,
    ]);
    let (outcome, children) = pump(&mut engine);
    assert_eq!(outcome, EngineOutcome::Suspended);
    let child_execution_id = children[0].child_execution_id.clone();
    reject(
        &mut engine,
        HostResponse::ChildFailed {
            message: "child broke".into(),
            child_execution_id: "child:other".into(),
            branch: None,
        },
        &child_execution_id,
    );
    engine
        .apply_host_response(HostResponse::ChildFailed {
            message: "child broke".into(),
            child_execution_id,
            branch: None,
        })
        .unwrap();
    assert_eq!(
        finish(&mut engine),
        EngineOutcome::Failed {
            message: "child broke".into()
        }
    );
}

#[test]
fn caught_child_failure_resumes_at_the_handler() {
    let mut engine = start(vec![
        Instruction::PushTry {
            catch: Pc(5),
            finally: None,
        },
        load("child"),
        Instruction::Invoke { arg_count: 0 },
        Instruction::PopTry,
        Instruction::Jump { target: Pc(6) },
        Instruction::Return,
        Instruction::LoadConst {
            value: ConstValue::Null,
        },
        Instruction::Return,
    ]);
    let (outcome, children) = pump(&mut engine);
    assert_eq!(outcome, EngineOutcome::Suspended);
    engine
        .apply_host_response(HostResponse::ChildFailed {
            message: "child broke".into(),
            child_execution_id: children[0].child_execution_id.clone(),
            branch: None,
        })
        .unwrap();
    assert_eq!(
        finish(&mut engine),
        EngineOutcome::Completed {
            result: Value::String("child broke".into())
        }
    );
}

fn two_invokes(join: Instruction) -> Vec<Instruction> {
    vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        load("left"),
        Instruction::Invoke { arg_count: 0 },
        load("right"),
        Instruction::Invoke { arg_count: 0 },
        join,
        Instruction::Return,
    ]
}

#[test]
fn join_child_delivery_must_match_the_registered_branch() {
    let program = artifact(two_invokes(Instruction::JoinAll));
    let mut engine = Engine::start(program.clone(), "exec", &EngineCaps::current()).unwrap();
    let (outcome, children) = pump(&mut engine);
    assert_eq!(outcome, EngineOutcome::Suspended);
    assert_eq!(children.len(), 2);
    reject(
        &mut engine,
        HostResponse::ChildResult {
            value: Value::Number(1.0),
            child_execution_id: children[0].child_execution_id.clone(),
            branch: Some("missing".into()),
        },
        "unknown branch",
    );
    reject(
        &mut engine,
        HostResponse::ChildResult {
            value: Value::Number(1.0),
            child_execution_id: children[1].child_execution_id.clone(),
            branch: Some(children[0].invoke_id.clone()),
        },
        "does not match branch",
    );

    engine
        .apply_host_response(HostResponse::ChildResult {
            value: Value::Number(1.0),
            child_execution_id: children[0].child_execution_id.clone(),
            branch: Some(children[0].invoke_id.clone()),
        })
        .unwrap();
    assert_eq!(finish(&mut engine), EngineOutcome::Suspended);
    let join = engine.continuation().join.clone().unwrap();
    assert_eq!(join.state, JoinStatus::Active);
    assert_eq!(join.branches[0].phase, BranchPhase::Completed);
    assert_eq!(join.branches[1].phase, BranchPhase::Registered);

    engine
        .apply_host_response(HostResponse::ChildFailed {
            message: "right broke".into(),
            child_execution_id: children[1].child_execution_id.clone(),
            branch: Some(children[1].invoke_id.clone()),
        })
        .unwrap();
    let join = engine.continuation().join.clone().unwrap();
    assert_eq!(join.state, JoinStatus::Failed);
    assert_eq!(join.failure_branch, Some(1));
    assert_eq!(join.branches[0].phase, BranchPhase::Completed);
    assert_eq!(join.branches[1].phase, BranchPhase::Failed);
    assert_eq!(
        finish(&mut engine),
        EngineOutcome::Failed {
            message: "right broke".into()
        }
    );
}

#[test]
fn stale_child_result_for_a_detached_branch_is_ignored() {
    let program = artifact(two_invokes(Instruction::JoinAll));
    let mut engine = Engine::start(program.clone(), "exec", &EngineCaps::current()).unwrap();
    let (outcome, children) = pump(&mut engine);
    assert_eq!(outcome, EngineOutcome::Suspended);
    engine
        .apply_host_response(HostResponse::ChildFailed {
            message: "left broke".into(),
            child_execution_id: children[0].child_execution_id.clone(),
            branch: Some(children[0].invoke_id.clone()),
        })
        .unwrap();
    let saved = engine.continuation().clone();
    assert_eq!(
        saved.join.as_ref().unwrap().branches[1].phase,
        BranchPhase::Detached
    );
    let mut resumed = Engine::resume(program, saved, &EngineCaps::current()).unwrap();
    reject(
        &mut resumed,
        HostResponse::ChildResult {
            value: Value::Number(2.0),
            child_execution_id: "child:other".into(),
            branch: Some(children[1].invoke_id.clone()),
        },
        "does not match branch",
    );
    resumed
        .apply_host_response(HostResponse::ChildResult {
            value: Value::Number(2.0),
            child_execution_id: children[1].child_execution_id.clone(),
            branch: Some(children[1].invoke_id.clone()),
        })
        .unwrap();
    assert_eq!(
        resumed.continuation().join.as_ref().unwrap().state,
        JoinStatus::Failed
    );
    assert_eq!(
        finish(&mut engine),
        EngineOutcome::Failed {
            message: "left broke".into()
        }
    );
}

#[test]
fn race_child_failure_uses_the_existing_settlement_rule() {
    let program = artifact(two_invokes(Instruction::JoinAny));
    let mut engine = Engine::start(program.clone(), "exec", &EngineCaps::current()).unwrap();
    let (outcome, children) = pump(&mut engine);
    assert_eq!(outcome, EngineOutcome::Suspended);
    engine
        .apply_host_response(HostResponse::ChildResult {
            value: Value::Number(4.0),
            child_execution_id: children[0].child_execution_id.clone(),
            branch: Some(children[0].invoke_id.clone()),
        })
        .unwrap();
    let saved = engine.continuation().clone();
    assert_eq!(
        saved.join.as_ref().unwrap().branches[1].phase,
        BranchPhase::Detached
    );
    let mut resumed = Engine::resume(program, saved, &EngineCaps::current()).unwrap();
    resumed
        .apply_host_response(HostResponse::ChildFailed {
            message: "late".into(),
            child_execution_id: children[1].child_execution_id.clone(),
            branch: Some(children[1].invoke_id.clone()),
        })
        .unwrap();
    assert_eq!(
        resumed.continuation().join.as_ref().unwrap().state,
        JoinStatus::Succeeded
    );
    assert_eq!(
        finish(&mut engine),
        EngineOutcome::Completed {
            result: Value::Number(4.0)
        }
    );

    let mut failed = Engine::start(
        artifact(two_invokes(Instruction::JoinAny)),
        "exec",
        &EngineCaps::current(),
    )
    .unwrap();
    let (outcome, children) = pump(&mut failed);
    assert_eq!(outcome, EngineOutcome::Suspended);
    failed
        .apply_host_response(HostResponse::ChildFailed {
            message: "slow broke".into(),
            child_execution_id: children[1].child_execution_id.clone(),
            branch: Some(children[1].invoke_id.clone()),
        })
        .unwrap();
    let join = failed.continuation().join.clone().unwrap();
    assert_eq!(join.state, JoinStatus::Failed);
    assert_eq!(join.failure_branch, Some(1));
    assert_eq!(join.branches[0].phase, BranchPhase::Detached);
    assert_eq!(
        finish(&mut failed),
        EngineOutcome::Failed {
            message: "slow broke".into()
        }
    );
}
