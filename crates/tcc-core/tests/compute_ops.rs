use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{
    Artifact, ConstValue, EngineCaps, Envelope, FuncId, Function, Instruction, LocalId, Program,
    ENGINE_FORMAT_VERSION, LANGUAGE_SEMANTICS_PY, LANGUAGE_SEMANTICS_TS,
};
use tcc_state::Value;

fn artifact(language: &str, functions: Vec<Function>) -> Artifact {
    Artifact {
        envelope: Envelope {
            artifact_hash: "compute".into(),
            frontend_id: "test".into(),
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
            functions,
        },
    }
}

fn function(
    id: u32,
    param_count: u32,
    local_count: u32,
    instructions: Vec<Instruction>,
) -> Function {
    let spans = vec![None; instructions.len()];
    Function {
        id: FuncId(id),
        name: if id == 0 {
            "run".into()
        } else {
            format!("f{id}")
        },
        param_count,
        local_count,
        param_defaults: Vec::new(),
        instructions,
        spans,
    }
}

fn num(value: f64) -> Instruction {
    Instruction::LoadConst {
        value: ConstValue::Number(value),
    }
}

fn run(language: &str, instructions: Vec<Instruction>) -> Result<Value, String> {
    run_functions(language, vec![function(0, 0, 0, instructions)])
}

fn run_functions(language: &str, functions: Vec<Function>) -> Result<Value, String> {
    let mut engine = Engine::start(
        artifact(language, functions),
        "compute",
        &EngineCaps::current(),
    )
    .map_err(|err| err.to_string())?;
    loop {
        match engine.run_until_host(256) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Completed { result } => return Ok(result),
            EngineOutcome::Failed { message } => return Err(message),
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn typescript_division_produces_canonical_nonfinite_numbers() {
    let inf = run(
        LANGUAGE_SEMANTICS_TS,
        vec![num(1.0), num(0.0), Instruction::Div, Instruction::Return],
    )
    .unwrap();
    assert_eq!(inf, Value::Number(f64::INFINITY));
    let nan = run(
        LANGUAGE_SEMANTICS_TS,
        vec![num(0.0), num(0.0), Instruction::Div, Instruction::Return],
    )
    .unwrap();
    assert_eq!(nan, Value::Number(f64::NAN));
    let sum = run(
        LANGUAGE_SEMANTICS_TS,
        vec![
            num(1.0),
            num(0.0),
            Instruction::Div,
            num(1.0),
            Instruction::Add,
            Instruction::Return,
        ],
    )
    .unwrap();
    assert_eq!(sum, Value::Number(f64::INFINITY));
}

#[test]
fn typescript_rejects_bool_plus_number_and_accepts_string_plus() {
    let err = run(
        LANGUAGE_SEMANTICS_TS,
        vec![
            Instruction::LoadConst {
                value: ConstValue::Bool(true),
            },
            num(1.0),
            Instruction::Add,
            Instruction::Return,
        ],
    )
    .unwrap_err();
    assert!(err.contains("unsupported"), "{err}");
    let text = run(
        LANGUAGE_SEMANTICS_TS,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("id:".into()),
            },
            num(3.0),
            Instruction::Add,
            Instruction::Return,
        ],
    )
    .unwrap();
    assert_eq!(text, Value::String("id:3".into()));
}

#[test]
fn remainder_follows_the_language() {
    let js = run(
        LANGUAGE_SEMANTICS_TS,
        vec![num(-7.0), num(3.0), Instruction::Rem, Instruction::Return],
    )
    .unwrap();
    assert_eq!(js, Value::Number(-1.0));
    let py = run(
        LANGUAGE_SEMANTICS_PY,
        vec![num(-7.0), num(3.0), Instruction::Rem, Instruction::Return],
    )
    .unwrap();
    assert_eq!(py, Value::Number(2.0));
}

#[test]
fn python_bool_adds_as_int_and_division_by_zero_raises() {
    let sum = run(
        LANGUAGE_SEMANTICS_PY,
        vec![
            Instruction::LoadConst {
                value: ConstValue::Bool(true),
            },
            num(1.0),
            Instruction::Add,
            Instruction::Return,
        ],
    )
    .unwrap();
    assert_eq!(sum, Value::Number(2.0));
    let err = run(
        LANGUAGE_SEMANTICS_PY,
        vec![num(1.0), num(0.0), Instruction::Div, Instruction::Return],
    )
    .unwrap_err();
    assert!(err.contains("ZeroDivisionError"), "{err}");
}

#[test]
fn nan_is_not_strictly_equal_to_nan() {
    let equal = run(
        LANGUAGE_SEMANTICS_TS,
        vec![
            num(0.0),
            num(0.0),
            Instruction::Div,
            num(0.0),
            num(0.0),
            Instruction::Div,
            Instruction::StrictEq,
            Instruction::Return,
        ],
    )
    .unwrap();
    assert_eq!(equal, Value::Bool(false));
}

#[test]
fn index_is_language_specific() {
    let build = vec![
        Instruction::NewArray,
        num(10.0),
        Instruction::ArrayPush,
        num(40.0),
        Instruction::ArrayPush,
        Instruction::StoreLocal { local: LocalId(0) },
        Instruction::LoadLocal { local: LocalId(0) },
        num(-1.0),
        Instruction::GetIndex,
        Instruction::Return,
    ];
    let ts = run_functions(
        LANGUAGE_SEMANTICS_TS,
        vec![function(0, 0, 1, build.clone())],
    )
    .unwrap();
    assert_eq!(ts, Value::Undefined);
    let py = run_functions(LANGUAGE_SEMANTICS_PY, vec![function(0, 0, 1, build)]).unwrap();
    assert_eq!(py, Value::Number(40.0));
}

#[test]
fn call_copies_argument_refs_and_returns_to_the_caller() {
    let helper = function(
        1,
        1,
        1,
        vec![
            Instruction::LoadLocal { local: LocalId(0) },
            num(1.0),
            Instruction::Add,
            Instruction::Return,
        ],
    );
    let entry = function(
        0,
        0,
        0,
        vec![
            num(2.0),
            Instruction::Call {
                func: FuncId(1),
                argc: 1,
            },
            Instruction::Return,
        ],
    );
    let result = run_functions(LANGUAGE_SEMANTICS_TS, vec![entry, helper]).unwrap();
    assert_eq!(result, Value::Number(3.0));
}

#[test]
fn structural_mutation_of_the_iterated_cell_is_a_runtime_error() {
    let err = run_functions(
        LANGUAGE_SEMANTICS_TS,
        vec![function(
            0,
            0,
            1,
            vec![
                Instruction::NewArray,
                num(1.0),
                Instruction::ArrayPush,
                Instruction::StoreLocal { local: LocalId(0) },
                Instruction::LoadLocal { local: LocalId(0) },
                Instruction::WatchIter,
                Instruction::LoadLocal { local: LocalId(0) },
                num(2.0),
                Instruction::ArrayPush,
                Instruction::Return,
            ],
        )],
    )
    .unwrap_err();
    assert!(err.contains("being iterated"), "{err}");
}
