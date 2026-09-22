use tcc_core::{
    decode_request, encode_request, ChildSpec, CoreError, Engine, EngineOutcome, HostRequest,
    HostResponse,
};
use tcc_ir::{
    Artifact, ConstValue, EngineCaps, Envelope, FuncId, Function, Instruction, LocalId, Program,
    ENGINE_FORMAT_VERSION,
};
use tcc_state::{ContinuationStatus, Value};

fn artifact(
    language: &str,
    param_count: u32,
    local_count: u32,
    instructions: Vec<Instruction>,
) -> Artifact {
    let spans = vec![None; instructions.len()];
    Artifact {
        envelope: Envelope {
            artifact_hash: "program-input".into(),
            frontend_id: "frontend".into(),
            frontend_version: "0.0.0".into(),
            language_semantics_version: language.into(),
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
                param_count,
                local_count,
                instructions,
                spans,
            }],
        },
    }
}

fn caps() -> EngineCaps {
    EngineCaps::current()
}

#[test]
fn typescript_pads_missing_arguments_and_ignores_extras() {
    let program = artifact(
        "ts.subset.v1",
        2,
        2,
        vec![
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Return,
        ],
    );
    let missing = Engine::start_with_args(program.clone(), "missing", &caps(), &[]).unwrap();
    assert_eq!(missing.continuation().frames[0].locals[0], Value::Undefined);
    assert_eq!(missing.continuation().frames[0].locals[1], Value::Undefined);

    let short = Engine::start_with_args(
        program.clone(),
        "short",
        &caps(),
        &[Value::String("a".into())],
    )
    .unwrap();
    assert_eq!(
        short.continuation().frames[0].locals[0],
        Value::String("a".into())
    );
    assert_eq!(short.continuation().frames[0].locals[1], Value::Undefined);

    let explicit =
        Engine::start_with_args(program.clone(), "null", &caps(), &[Value::Null]).unwrap();
    assert_eq!(explicit.continuation().frames[0].locals[0], Value::Null);
    assert_eq!(
        explicit.continuation().frames[0].locals[1],
        Value::Undefined
    );

    let extra = Engine::start_with_args(
        program,
        "extra",
        &caps(),
        &[
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ],
    )
    .unwrap();
    assert_eq!(extra.continuation().frames[0].locals.len(), 2);
    assert_eq!(
        extra.continuation().frames[0].locals[1],
        Value::String("b".into())
    );

    let no_param = artifact("ts.subset.v1", 0, 0, vec![Instruction::Return]);
    Engine::start_with_args(no_param, "ignored", &caps(), &[Value::Null]).unwrap();
}

#[test]
fn python_requires_exact_positional_arity() {
    let program = artifact(
        "py.subset.v1",
        2,
        2,
        vec![
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Return,
        ],
    );
    let short =
        Engine::start_with_args(program.clone(), "short", &caps(), &[Value::Null]).unwrap_err();
    assert!(matches!(short, CoreError::TypeError(message) if message.contains("missing")));
    let extra = Engine::start_with_args(
        program.clone(),
        "extra",
        &caps(),
        &[Value::Null, Value::Null, Value::Null],
    )
    .unwrap_err();
    assert!(matches!(extra, CoreError::TypeError(message) if message.contains("were given")));
    let exact = Engine::start_with_args(
        program,
        "exact",
        &caps(),
        &[Value::Null, Value::String("b".into())],
    )
    .unwrap();
    assert_eq!(exact.continuation().frames[0].locals[0], Value::Null);
    assert_eq!(
        exact.continuation().frames[0].locals[1],
        Value::String("b".into())
    );

    let no_param = artifact("py.subset.v1", 0, 0, vec![Instruction::Return]);
    let err = Engine::start_with_args(no_param, "none", &caps(), &[Value::Null]).unwrap_err();
    assert!(matches!(err, CoreError::TypeError(message) if message.contains("were given")));
}

#[test]
fn unused_argument_is_cleared_and_a_later_read_keeps_it() {
    let dead = artifact(
        "ts.subset.v1",
        1,
        1,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("go".into()),
            },
            Instruction::WaitForEvent,
            Instruction::Return,
        ],
    );
    let mut engine =
        Engine::start_with_args(dead, "dead", &caps(), &[Value::String("keep".into())]).unwrap();
    ack_until_suspended(&mut engine);
    assert_eq!(engine.continuation().frames[0].locals[0], Value::Undefined);

    let live = artifact(
        "ts.subset.v1",
        1,
        1,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("go".into()),
            },
            Instruction::WaitForEvent,
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Return,
        ],
    );
    let mut engine =
        Engine::start_with_args(live.clone(), "live", &caps(), &[Value::String("q".into())])
            .unwrap();
    ack_until_suspended(&mut engine);
    assert_eq!(
        engine.continuation().frames[0].locals[0],
        Value::String("q".into())
    );
    let saved = engine.continuation().clone();
    let mut resumed = Engine::resume(live, saved, &caps()).unwrap();
    match resumed.run_until_host(16) {
        EngineOutcome::Suspended => resumed
            .apply_host_response(HostResponse::EventPayload {
                value: Value::String("ok".into()),
                branch: None,
            })
            .unwrap(),
        other => panic!("expected the wait to still be suspended: {other:?}"),
    }
    let result = finish(&mut resumed);
    assert_eq!(result, Value::String("q".into()));
}

#[test]
fn invoke_args_keep_source_order_and_omit_an_empty_vector() {
    let none = child_request(Vec::new());
    let text = encode_request(&none).unwrap();
    assert!(!text.contains("\"args\""));
    assert_eq!(decode_request(&text).unwrap(), none);

    let explicit = child_request(vec![Value::Null]);
    let text = encode_request(&explicit).unwrap();
    assert!(text.contains("\"args\":[{\"t\":\"null\"}]"));

    let program = artifact(
        "ts.subset.v1",
        0,
        0,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("a".into()),
            },
            Instruction::LoadConst {
                value: ConstValue::String("b".into()),
            },
            Instruction::LoadConst {
                value: ConstValue::String("child".into()),
            },
            Instruction::Invoke { arg_count: 2 },
            Instruction::Return,
        ],
    );
    let mut engine = Engine::start(program, "parent", &caps()).unwrap();
    let args = loop {
        match engine.run_until_host(32) {
            EngineOutcome::Host(HostRequest::CreateChild { child }) => break child.args,
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(
        args,
        vec![Value::String("a".into()), Value::String("b".into())]
    );
}

#[test]
fn invoke_consumes_exactly_arg_count_values() {
    let counted = drive_invoke(vec![
        load_string("sentinel"),
        load_string("a"),
        load_string("b"),
        load_string("child"),
        Instruction::Invoke { arg_count: 2 },
    ]);
    assert_eq!(
        counted.args,
        vec![Value::String("a".into()), Value::String("b".into())]
    );
    assert_eq!(counted.stack, vec![Value::String("sentinel".into())]);

    let zero = drive_invoke(vec![
        load_string("sentinel"),
        load_string("child"),
        Instruction::Invoke { arg_count: 0 },
    ]);
    assert!(zero.args.is_empty());
    assert_eq!(zero.stack, vec![Value::String("sentinel".into())]);
}

#[test]
fn resume_does_not_rebind_parameter_slots() {
    for language in ["ts.subset.v1", "py.subset.v1"] {
        let program = artifact(
            language,
            1,
            1,
            vec![
                Instruction::LoadLocal { local: LocalId(0) },
                Instruction::Return,
            ],
        );
        let started = Engine::start_with_args(
            program.clone(),
            "start",
            &caps(),
            &[Value::String("original".into())],
        )
        .unwrap();
        assert_eq!(
            started.continuation().frames[0].locals[0],
            Value::String("original".into())
        );
        let mut saved = started.continuation().clone();
        saved.frames[0].locals[0] = Value::String("checkpoint".into());
        let mut resumed = Engine::resume(program, saved, &caps()).unwrap();
        assert_eq!(
            resumed.continuation().frames[0].locals[0],
            Value::String("checkpoint".into())
        );
        assert_eq!(finish(&mut resumed), Value::String("checkpoint".into()));
    }
}

#[test]
fn invoke_arg_count_past_the_stack_underflows() {
    let program = artifact(
        "ts.subset.v1",
        0,
        0,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("child".into()),
            },
            Instruction::Invoke { arg_count: 1 },
        ],
    );
    let mut engine = Engine::start(program, "under", &caps()).unwrap();
    match engine.run_until_host(8) {
        EngineOutcome::Failed { message } => assert!(message.contains("underflow"), "{message}"),
        other => panic!("unexpected {other:?}"),
    }
}

struct InvokeYield {
    args: Vec<Value>,
    stack: Vec<Value>,
}

fn load_string(text: &str) -> Instruction {
    Instruction::LoadConst {
        value: ConstValue::String(text.into()),
    }
}

fn drive_invoke(instructions: Vec<Instruction>) -> InvokeYield {
    let program = artifact("ts.subset.v1", 0, 0, instructions);
    let mut engine = Engine::start(program, "parent", &caps()).unwrap();
    loop {
        match engine.run_until_host(32) {
            EngineOutcome::Host(HostRequest::CreateChild { child }) => {
                return InvokeYield {
                    args: child.args,
                    stack: engine.continuation().stack.clone(),
                };
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

fn child_request(args: Vec<Value>) -> HostRequest {
    HostRequest::CreateChild {
        child: ChildSpec {
            invoke_id: "parent:invoke:0".into(),
            child_execution_id: "child:parent:invoke:0".into(),
            program_name: "child".into(),
            args,
        },
    }
}

fn ack_until_suspended(engine: &mut Engine) {
    loop {
        match engine.run_until_host(32) {
            EngineOutcome::Suspended => return,
            EngineOutcome::Host(HostRequest::RegisterWait { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

fn finish(engine: &mut Engine) -> Value {
    loop {
        match engine.run_until_host(32) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Completed { result } => {
                assert_eq!(engine.continuation().status, ContinuationStatus::Completed);
                return result;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
