use std::collections::HashMap;
use std::path::PathBuf;

use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{EngineCaps, Instruction};
use tcc_rust_frontend::compile;
use tcc_state::{decode_continuation, encode_continuation, Value};

fn source(name: &str) -> String {
    let topic = match name {
        "move_struct.rs" | "move_string.rs" | "helper_move.rs" | "closure_capture.rs" => {
            "ownership"
        }
        "copy_f64.rs" => "values",
        "update_struct.rs" | "construct_job.rs" => "structs",
        "match_enum.rs" | "match_option.rs" | "result_question.rs" | "owned_enums.rs"
        | "result_job.rs" => "matching",
        "vec_values.rs" | "vec_jobs.rs" | "vec_strings.rs" => "collections",
        "range_for.rs" | "while_loop.rs" => "iteration",
        "helper_call.rs" => "helpers",
        "closure_move.rs" => "closures",
        "effect_then_wait.rs"
        | "invoke_args.rs"
        | "join_pair.rs"
        | "race_pair.rs"
        | "join_strings.rs" => "durability",
        other => panic!("no topic for {other}"),
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/ordinary/rust")
        .join(topic)
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn ok_number(value: f64) -> Value {
    object(vec![
        ("$tag", Value::String("Ok".into())),
        ("$0", Value::Number(value)),
    ])
}

fn ok_string(value: &str) -> Value {
    object(vec![
        ("$tag", Value::String("Ok".into())),
        ("$0", Value::String(value.into())),
    ])
}

fn run(
    name: &str,
    args: &[Value],
    effects: &[(&str, Value)],
    event: Option<Value>,
    child: Option<Value>,
) -> Value {
    let artifact = compile(&source(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
    let mut engine = Engine::start_with_args(artifact, name, &EngineCaps::current(), args).unwrap();
    let effects: HashMap<&str, Value> = effects.iter().cloned().collect();
    for _ in 0..80 {
        match engine.run_until_host(10_000) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::RunEffect { key, .. }) => {
                let value = effects
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or_else(|| panic!("{name}: missing effect {key}"));
                engine
                    .apply_host_response(HostResponse::EffectResult { value })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::RegisterWait { .. })
            | EngineOutcome::Host(HostRequest::PersistEffect { .. })
            | EngineOutcome::Host(HostRequest::CreateChild { .. })
            | EngineOutcome::Host(HostRequest::RegisterTimer { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Suspended => {
                if let Some(value) = child.clone() {
                    engine
                        .apply_host_response(HostResponse::ChildResult {
                            value,
                            branch: None,
                        })
                        .unwrap();
                } else if let Some(value) = event.clone() {
                    engine
                        .apply_host_response(HostResponse::EventPayload {
                            value,
                            branch: None,
                        })
                        .unwrap();
                } else {
                    panic!("{name}: suspended");
                }
            }
            EngineOutcome::Completed { result } => return result,
            other => panic!("{name}: {other:?}"),
        }
    }
    panic!("{name} did not complete");
}

#[test]
fn ordinary_rust_programs_match_their_results() {
    assert_eq!(
        run(
            "move_struct.rs",
            &[object(vec![("count", Value::Number(2.0))])],
            &[],
            Some(Value::Bool(true)),
            None,
        ),
        ok_number(3.0)
    );
    assert_eq!(
        run(
            "copy_f64.rs",
            &[Value::Number(10.0)],
            &[],
            Some(Value::Bool(true)),
            None,
        ),
        ok_number(10.0)
    );
    assert_eq!(
        run(
            "move_string.rs",
            &[Value::String("hello".into())],
            &[],
            Some(Value::Bool(true)),
            None,
        ),
        ok_string("hello")
    );
    assert_eq!(
        run(
            "update_struct.rs",
            &[object(vec![("count", Value::Number(0.0))])],
            &[],
            Some(Value::Bool(true)),
            None,
        ),
        ok_number(3.0)
    );
    assert_eq!(
        run(
            "match_enum.rs",
            &[object(vec![
                ("$tag", Value::String("Ready".into())),
                ("count", Value::Number(4.0)),
            ])],
            &[],
            None,
            None,
        ),
        Value::Number(4.0)
    );
    assert_eq!(
        run(
            "match_option.rs",
            &[object(vec![
                ("$tag", Value::String("Some".into())),
                ("$0", Value::Number(5.0)),
            ])],
            &[],
            None,
            None,
        ),
        Value::Number(5.0)
    );
    assert_eq!(
        run("result_question.rs", &[Value::Number(4.0)], &[], None, None),
        ok_number(5.0)
    );
    assert_eq!(
        run("vec_values.rs", &[], &[], None, None),
        Value::Number(42.0)
    );
    assert_eq!(
        run("range_for.rs", &[], &[], None, None),
        Value::Number(6.0)
    );
    assert_eq!(
        run("while_loop.rs", &[], &[], None, None),
        Value::Number(3.0)
    );
    assert_eq!(
        run("helper_call.rs", &[Value::Number(5.0)], &[], None, None),
        Value::Number(10.0)
    );
    assert_eq!(
        run(
            "helper_move.rs",
            &[object(vec![("count", Value::Number(4.0))])],
            &[],
            None,
            None,
        ),
        Value::Number(4.0)
    );
    assert_eq!(
        run("closure_move.rs", &[Value::Number(3.0)], &[], None, None),
        Value::Number(5.0)
    );
    assert_eq!(
        run(
            "closure_capture.rs",
            &[Value::String("hello".into())],
            &[],
            None,
            None,
        ),
        Value::String("hello".into())
    );
    assert_eq!(
        run(
            "effect_then_wait.rs",
            &[],
            &[("left", Value::Number(2.0))],
            Some(Value::Bool(true)),
            None,
        ),
        ok_number(5.0)
    );
    assert_eq!(
        run("invoke_args.rs", &[], &[], None, Some(Value::Number(7.0)),),
        ok_number(7.0)
    );
    assert_eq!(
        run(
            "join_pair.rs",
            &[],
            &[("a", Value::Number(1.0)), ("b", Value::Number(2.0))],
            None,
            None,
        ),
        ok_number(3.0)
    );
    assert_eq!(
        run(
            "race_pair.rs",
            &[],
            &[("a", Value::Number(1.0)), ("b", Value::Number(2.0))],
            None,
            None,
        ),
        ok_number(1.0)
    );
    assert_eq!(
        run("construct_job.rs", &[], &[], None, None),
        Value::Number(42.0)
    );
    assert_eq!(
        run("vec_jobs.rs", &[], &[], None, None),
        Value::Number(10.0)
    );
    assert_eq!(
        run("vec_strings.rs", &[], &[], None, None),
        Value::Number(1.0)
    );
    assert_eq!(
        run("owned_enums.rs", &[], &[], None, None),
        Value::Number(43.0)
    );
    assert_eq!(
        run("result_job.rs", &[Value::Number(42.0)], &[], None, None),
        ok_number(42.0)
    );
    assert_eq!(
        run("result_job.rs", &[Value::Number(0.0)], &[], None, None),
        object(vec![
            ("$tag", Value::String("Err".into())),
            ("$0", Value::String("missing".into())),
        ])
    );
    assert_eq!(
        run(
            "join_strings.rs",
            &[],
            &[
                ("a", Value::String("left".into())),
                ("b", Value::String("right".into())),
            ],
            None,
            None,
        ),
        object(vec![
            ("$tag", Value::String("Ok".into())),
            (
                "$0",
                Value::Array(vec![
                    Value::String("left".into()),
                    Value::String("right".into()),
                ]),
            ),
        ])
    );
}

#[test]
fn invoke_flattens_the_tuple_into_the_argument_vector() {
    let artifact = compile(&source("invoke_args.rs")).unwrap();
    let count = artifact.program.functions[0]
        .instructions
        .iter()
        .find_map(|instruction| match instruction {
            tcc_ir::Instruction::Invoke { arg_count } => Some(*arg_count),
            _ => None,
        });
    assert_eq!(count, Some(2));
}

#[test]
fn diagnostics_reject_borrows_macros_generics_and_integers() {
    let macro_error =
        compile("macro_rules! m { () => {}; } async fn run() -> f64 { 1.0 }").unwrap_err();
    assert!(macro_error.message.contains("macro"), "{macro_error}");
    let generic =
        compile("fn id<T>(value: T) -> T { value } async fn run() -> f64 { 1.0 }").unwrap_err();
    assert!(generic.message.contains("generic"), "{generic}");
    let integer = compile("async fn run(value: i32) -> i32 { value }").unwrap_err();
    assert!(integer.message.contains("integer"), "{integer}");
    let capture =
        compile("async fn run(name: String) -> String { let read = || name; read() }").unwrap_err();
    assert!(capture.message.contains("move"), "{capture}");
    let direct = compile("struct A { a: A } async fn run() -> f64 { 1.0 }").unwrap_err();
    assert!(direct.message.contains("recursive"), "{direct}");
    let indirect = compile(
        "struct A { xs: Vec<Option<Result<B, String>>> } struct B { a: A } async fn run() -> f64 { 1.0 }",
    )
    .unwrap_err();
    assert!(indirect.message.contains("recursive"), "{indirect}");
    let nested = compile(
        "struct Job { count: f64 } async fn run(value: Option<Job>) -> f64 { match value { Some(Job { count }) => count, None => 0.0 } }",
    )
    .unwrap_err();
    assert!(nested.message.contains("nested"), "{nested}");
    let index = compile(
        "async fn run() -> String { let mut xs: Vec<String> = Vec::new(); xs.push(String::from(\"a\")); xs[0.0] }",
    )
    .unwrap_err();
    assert!(index.message.contains("borrow"), "{index}");
    let again = compile(
        "struct Job { name: String, score: f64 } async fn run() -> f64 { let job = Job { name: String::from(\"a\"), score: 1.0 }; let _name = job.name; let _again = job.name; job.score }",
    )
    .unwrap_err();
    assert!(again.message.contains("moved"), "{again}");
    let pair =
        compile("enum State { Pair(String, f64) } async fn run() -> f64 { 1.0 }").unwrap_err();
    assert!(pair.message.contains("tuple"), "{pair}");
    let local =
        compile("fn effect(x: f64) -> f64 { x * 2.0 } async fn run() -> f64 { effect(3.0) }")
            .unwrap();
    assert!(
        !local.program.functions.iter().any(|function| function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Effect { .. }))),
        "a local effect helper is not a durable call"
    );
    let glob = compile("use tcc_rust_prelude::*; async fn run() -> f64 { 1.0 }").unwrap_err();
    assert!(glob.message.contains("glob"), "{glob}");
    let qualified = compile(
        "async fn run() -> Result<f64, String> { trigora::effect(\"search\", || 1.0).await? }",
    )
    .unwrap_err();
    assert!(qualified.message.contains("import"), "{qualified}");
    let shadow =
        compile("use tcc_rust_prelude::effect; async fn run(effect: f64) -> f64 { effect }")
            .unwrap_err();
    assert!(shadow.message.contains("shadow"), "{shadow}");
    let local_shadow = compile(
        "use tcc_rust_prelude::sleep; async fn run() -> Result<(), String> { let sleep = 1.0; sleep(sleep).await }",
    )
    .unwrap_err();
    assert!(local_shadow.message.contains("shadow"), "{local_shadow}");
    let bare_capture = compile(
        "use tcc_rust_prelude::effect; async fn run(query: String) -> Result<String, String> { effect(\"search\", || query).await? }",
    )
    .unwrap_err();
    assert!(bare_capture.message.contains("move"), "{bare_capture}");
    let moved_again = compile(
        "use tcc_rust_prelude::effect; async fn run() -> Result<String, String> { let query = String::from(\"hello\"); let found = effect(\"search\", move || query).await?; let _again = query; Ok(found) }",
    )
    .unwrap_err();
    assert!(moved_again.message.contains("moved"), "{moved_again}");
}

#[test]
fn rust_effects_are_imported_and_always_carry_input() {
    let aliased = compile(
        "use tcc_rust_prelude::effect as fx; async fn run() -> Result<f64, String> { let value = fx(\"search\", || 1.0).await?; Ok(value) }",
    )
    .unwrap();
    assert!(aliased.program.functions[0]
        .instructions
        .iter()
        .any(|instruction| matches!(instruction, Instruction::Effect { has_input: true })));
    let empty = compile(
        "use tcc_rust_prelude::effect; async fn run() -> Result<f64, String> { let value = effect(\"search\", || 1.0).await?; Ok(value) }",
    )
    .unwrap();
    let instructions = &empty.program.functions[0].instructions;
    let effect_at = instructions
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Effect { has_input: true }))
        .expect("zero-capture effect");
    assert!(instructions[..effect_at]
        .iter()
        .any(|instruction| matches!(instruction, Instruction::NewObject)));
    let copied = compile(
        "use tcc_rust_prelude::effect; async fn run(limit: f64) -> Result<f64, String> { let value = effect(\"search\", move || limit).await?; Ok(value + limit) }",
    )
    .unwrap();
    assert!(copied.program.functions[0]
        .instructions
        .iter()
        .any(|instruction| {
            matches!(instruction, Instruction::SetProp { key } if key == "limit")
        }));
}

#[test]
fn moved_effect_capture_survives_resume_after_the_effect_checkpoint() {
    let source = r#"
        use tcc_rust_prelude::effect;
        async fn run() -> Result<String, String> {
            let query = String::from("hello");
            let found = effect("search", move || query).await?;
            Ok(found)
        }
    "#;
    let artifact = compile(source).unwrap();
    let mut engine = Engine::start(artifact.clone(), "search", &EngineCaps::current()).unwrap();
    let request = loop {
        match engine.run_until_host(10_000) {
            EngineOutcome::Host(HostRequest::RunEffect { key, input, .. }) => {
                break HostRequest::RunEffect {
                    key,
                    input,
                    idempotency_key: String::new(),
                };
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            other => panic!("before effect: {other:?}"),
        }
    };
    let HostRequest::RunEffect { key, input, .. } = request else {
        unreachable!();
    };
    assert_eq!(key, "search");
    assert_eq!(
        input,
        Value::Object(std::collections::BTreeMap::from([(
            "query".into(),
            Value::String("hello".into()),
        )]))
    );
    assert_eq!(
        engine.continuation().frames[0].locals[0],
        Value::Undefined,
        "the moved capture is dead at the yield"
    );
    engine
        .apply_host_response(HostResponse::EffectResult {
            value: Value::String("found".into()),
        })
        .unwrap();
    loop {
        match engine.run_until_host(10_000) {
            EngineOutcome::Host(HostRequest::PersistEffect { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
                break;
            }
            other => panic!("checkpoint: {other:?}"),
        }
    }
    assert_eq!(engine.continuation().frames[0].locals[0], Value::Undefined);
    let saved = encode_continuation(engine.continuation()).unwrap();
    let mut resumed = Engine::resume(
        artifact,
        decode_continuation(&saved).unwrap(),
        &EngineCaps::current(),
    )
    .unwrap();
    let mut effects = 0;
    let result = loop {
        match resumed.run_until_host(10_000) {
            EngineOutcome::Host(HostRequest::RunEffect { .. }) => effects += 1,
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                resumed
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistEffect { .. }) => {
                resumed.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Completed { result } => break result,
            other => panic!("resume: {other:?}"),
        }
    };
    assert_eq!(effects, 0, "a completed effect is not requested again");
    assert_eq!(
        result,
        Value::Object(std::collections::BTreeMap::from([
            ("$tag".into(), Value::String("Ok".into())),
            ("$0".into(), Value::String("found".into())),
        ]))
    );
    assert!(resumed.continuation().frames.is_empty());
}

#[test]
fn use_after_move_is_not_lowered_when_cargo_check_rejects_it() {
    let cargo = std::env::var("CARGO").unwrap();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = std::env::temp_dir().join(format!("tcc-rust-negative-{}", std::process::id()));
    let status = std::process::Command::new(cargo)
        .current_dir(workspace)
        .env("CARGO_TARGET_DIR", &target)
        .args([
            "check",
            "-p",
            "tcc-rust-fixtures",
            "--features",
            "negative",
            "--offline",
        ])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "cargo check should reject use after move"
    );
}
