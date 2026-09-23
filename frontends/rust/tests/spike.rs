use std::path::PathBuf;
use std::process::Command;

use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{EngineCaps, Instruction, LocalId};
use tcc_rust_frontend::compile;
use tcc_state::{export_value, ContinuationStatus, HeapCell, Value};

fn source(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn artifact(name: &str) -> tcc_ir::Artifact {
    compile(&source(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn load_count(artifact: &tcc_ir::Artifact, local: u32) -> usize {
    artifact.program.functions[0]
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction,
                Instruction::LoadLocal {
                    local: LocalId(id)
                } if *id == local
            )
        })
        .count()
}

fn drive_to_suspend(engine: &mut Engine) {
    for _ in 0..40 {
        match engine.run_until_host(10_000) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::RegisterWait { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Suspended => {
                assert_eq!(engine.continuation().frames.len(), 1);
                assert_eq!(engine.continuation().status, ContinuationStatus::Suspended);
                return;
            }
            other => panic!("unexpected before suspend: {other:?}"),
        }
    }
    panic!("did not suspend");
}

fn finish(engine: &mut Engine, event: Value) -> Value {
    engine
        .apply_host_response(HostResponse::EventPayload {
            value: event,
            branch: None,
        })
        .unwrap();
    for _ in 0..40 {
        match engine.run_until_host(10_000) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Completed { result } => return result,
            other => panic!("unexpected after event: {other:?}"),
        }
    }
    panic!("did not complete");
}

fn run_pure(name: &str, arg: Value) -> Value {
    let mut engine =
        Engine::start_with_args(artifact(name), "spike", &EngineCaps::current(), &[arg]).unwrap();
    for _ in 0..40 {
        match engine.run_until_host(10_000) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Completed { result } => return result,
            other => panic!("{name}: {other:?}"),
        }
    }
    panic!("{name} did not complete");
}

fn tagged(value: &Value) -> (String, Value) {
    let Value::Object(fields) = value else {
        panic!("expected object, got {value:?}");
    };
    let tag = match fields.get("$tag") {
        Some(Value::String(tag)) => tag.clone(),
        other => panic!("missing tag: {other:?}"),
    };
    let payload = fields.get("$0").cloned().unwrap_or(Value::Undefined);
    (tag, payload)
}

fn refs(engine: &Engine) -> Vec<u32> {
    engine.continuation().frames[0]
        .locals
        .iter()
        .chain(engine.continuation().stack.iter())
        .filter_map(|value| match value {
            Value::Ref(id) => Some(*id),
            _ => None,
        })
        .collect()
}

fn field(engine: &Engine, id: u32, key: &str) -> Value {
    match &engine.continuation().heap[id as usize] {
        HeapCell::Object(fields) => fields.get(key).cloned().unwrap(),
        other => panic!("heap {id} is {other:?}"),
    }
}

#[test]
fn cargo_check_accepts_the_fixtures() {
    let cargo = std::env::var("CARGO").unwrap();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = std::env::temp_dir().join(format!("tcc-rust-fixtures-{}", std::process::id()));
    let status = Command::new(cargo)
        .current_dir(workspace)
        .env("CARGO_TARGET_DIR", &target)
        .args(["check", "-p", "tcc-rust-fixtures", "--offline"])
        .status()
        .unwrap();
    assert!(status.success(), "cargo check of the spike fixtures failed");
}

#[test]
fn borrow_is_rejected() {
    let error = compile(&source("borrow.rs")).unwrap_err();
    assert!(
        error.message.contains("references are outside this subset"),
        "{}",
        error.message
    );
    assert!(error.line >= 1, "diagnostic line {}", error.line);
}

#[test]
fn user_copy_is_rejected() {
    let error = compile(
        r#"
        #[derive(Copy, Clone)]
        struct Job { count: f64 }
        async fn run(job: Job) -> f64 { job.count }
        "#,
    )
    .unwrap_err();
    assert!(
        error
            .message
            .contains("user-defined Copy and Clone are outside this subset"),
        "{}",
        error.message
    );
}

#[test]
fn owned_job_survives_the_boundary() {
    let mut engine = Engine::start_with_args(
        artifact("job.rs"),
        "job",
        &EngineCaps::current(),
        &[object(vec![("count", Value::Number(0.0))])],
    )
    .unwrap();
    drive_to_suspend(&mut engine);
    let saved = engine.continuation().clone();
    let approved = finish(&mut engine, Value::Bool(true));
    let (tag, payload) = tagged(&approved);
    assert_eq!(tag, "Ok");
    assert_eq!(payload, Value::Number(3.0));

    let mut resumed = Engine::resume(artifact("job.rs"), saved, &EngineCaps::current()).unwrap();
    let again = finish(&mut resumed, Value::Bool(false));
    let (tag, payload) = tagged(&again);
    assert_eq!(tag, "Err");
    assert_eq!(payload, Value::String("rejected".into()));
}

#[test]
fn move_clears_the_source_and_keeps_one_ref() {
    let program = artifact("move_job.rs");
    assert_eq!(load_count(&program, 0), 1);
    let mut engine = Engine::start_with_args(
        program,
        "move",
        &EngineCaps::current(),
        &[object(vec![("count", Value::Number(2.0))])],
    )
    .unwrap();
    drive_to_suspend(&mut engine);
    assert_eq!(engine.continuation().frames[0].locals[0], Value::Undefined);
    let live = refs(&engine);
    assert_eq!(
        live.len(),
        1,
        "locals {:?}",
        engine.continuation().frames[0].locals
    );
    assert_eq!(field(&engine, live[0], "count"), Value::Number(3.0));
    let saved = engine.continuation().clone();
    let result = finish(&mut engine, Value::Bool(true));
    let (tag, payload) = tagged(&result);
    assert_eq!((tag.as_str(), payload), ("Ok", Value::Number(3.0)));

    let mut resumed =
        Engine::resume(artifact("move_job.rs"), saved, &EngineCaps::current()).unwrap();
    assert_eq!(resumed.continuation().frames.len(), 1);
    let again = finish(&mut resumed, Value::Bool(true));
    let (tag, payload) = tagged(&again);
    assert_eq!((tag.as_str(), payload), ("Ok", Value::Number(3.0)));
}

#[test]
fn f64_copy_does_not_share_storage() {
    let mut engine = Engine::start_with_args(
        artifact("copy_f64.rs"),
        "copy",
        &EngineCaps::current(),
        &[Value::Number(10.0)],
    )
    .unwrap();
    drive_to_suspend(&mut engine);
    let locals = &engine.continuation().frames[0].locals;
    assert_eq!(locals[0], Value::Number(10.0));
    assert!(
        locals.iter().any(|value| value == &Value::Number(11.0)),
        "{locals:?}"
    );
    assert!(refs(&engine).is_empty(), "f64 locals are numbers, not refs");
    let saved = engine.continuation().clone();
    let result = finish(&mut engine, Value::Bool(true));
    let (tag, payload) = tagged(&result);
    assert_eq!((tag.as_str(), payload), ("Ok", Value::Number(10.0)));
    let mut resumed =
        Engine::resume(artifact("copy_f64.rs"), saved, &EngineCaps::current()).unwrap();
    let again = finish(&mut resumed, Value::Bool(true));
    let (tag, payload) = tagged(&again);
    assert_eq!((tag.as_str(), payload), ("Ok", Value::Number(10.0)));
}

#[test]
fn string_move_clears_the_source() {
    let program = artifact("move_string.rs");
    assert_eq!(load_count(&program, 0), 1);
    let mut engine = Engine::start_with_args(
        program,
        "string",
        &EngineCaps::current(),
        &[Value::String("hello".into())],
    )
    .unwrap();
    drive_to_suspend(&mut engine);
    assert_eq!(engine.continuation().frames[0].locals[0], Value::Undefined);
    assert!(engine.continuation().frames[0]
        .locals
        .iter()
        .any(|value| value == &Value::String("hello".into())));
    let result = finish(&mut engine, Value::Bool(true));
    let (tag, payload) = tagged(&result);
    assert_eq!(tag, "Ok");
    assert_eq!(payload, Value::String("hello".into()));
}

#[test]
fn enum_and_option_match() {
    assert_eq!(
        run_pure(
            "match_state.rs",
            object(vec![
                ("$tag", Value::String("Ready".into())),
                ("count", Value::Number(4.0)),
            ]),
        ),
        Value::Number(4.0)
    );
    assert_eq!(
        run_pure(
            "match_state.rs",
            object(vec![("$tag", Value::String("Done".into()))]),
        ),
        Value::Number(0.0)
    );
    assert_eq!(
        run_pure(
            "match_option.rs",
            object(vec![
                ("$tag", Value::String("Some".into())),
                ("$0", Value::Number(5.0)),
            ]),
        ),
        Value::Number(5.0)
    );
    assert_eq!(
        run_pure(
            "match_option.rs",
            object(vec![("$tag", Value::String("None".into()))]),
        ),
        Value::Number(0.0)
    );
}

#[test]
fn resume_exports_the_same_numbers_the_heap_held() {
    let mut engine = Engine::start_with_args(
        artifact("copy_f64.rs"),
        "export",
        &EngineCaps::current(),
        &[Value::Number(10.0)],
    )
    .unwrap();
    drive_to_suspend(&mut engine);
    let exported = export_value(
        &engine.continuation().heap,
        &engine.continuation().frames[0].locals[0],
    )
    .unwrap();
    assert_eq!(exported, Value::Number(10.0));
}
