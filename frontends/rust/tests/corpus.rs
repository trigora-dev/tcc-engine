use std::collections::HashMap;
use std::path::PathBuf;

use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::EngineCaps;
use tcc_rust_frontend::compile;
use tcc_state::Value;

fn source(name: &str) -> String {
    let topic = match name {
        "move_struct.rs" | "move_string.rs" | "helper_move.rs" | "closure_capture.rs" => {
            "ownership"
        }
        "copy_f64.rs" => "values",
        "update_struct.rs" => "structs",
        "match_enum.rs" | "match_option.rs" | "result_question.rs" => "matching",
        "vec_values.rs" => "collections",
        "range_for.rs" | "while_loop.rs" => "iteration",
        "helper_call.rs" => "helpers",
        "closure_move.rs" => "closures",
        "effect_then_wait.rs" | "invoke_args.rs" | "join_pair.rs" | "race_pair.rs" => "durability",
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
