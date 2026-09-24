use super::*;
use tcc_core::{Engine as NativeEngine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::encode_artifact;
use tcc_ir::gen::{generate, seed_count};
use tcc_ir::{ConstValue, Envelope, FuncId, Function, Instruction, LocalId, Pc, Program};
use tcc_state::Value;

#[test]
fn exports_current_format_version() {
    assert_eq!(tcc_engine_format_version(), ENGINE_FORMAT_VERSION);
    assert_eq!(ENGINE_FORMAT_VERSION, 1);
}

#[test]
fn c_abi_completes_sdk_first_example() {
    let artifact = encode_artifact(&Artifact::sdk_first_example("sdk-first")).unwrap();
    start(&artifact, "first");

    let first = run(32);
    assert!(first.contains("\"type\":\"run_effect\""));
    assert!(first.contains("\"idempotency_key\":\"first:generate\""));
    apply(r#"{"type":"effect_result","value":{"t":"number","v":42}}"#);

    let persist_effect = run(32);
    assert!(persist_effect.contains("\"type\":\"persist_effect\""));
    apply(r#"{"type":"ack"}"#);

    let checkpoint = run(32);
    assert!(checkpoint.contains("\"type\":\"persist_checkpoint\""));
    assert!(checkpoint.contains("\"kind\":\"snapshot\""));
    assert!(checkpoint.contains("\"materialize\":true"));
    apply(r#"{"type":"persist_confirmed","revision":1}"#);

    let wait = run(32);
    assert!(wait.contains("\"type\":\"register_wait\""));
    assert!(wait.contains("\"wait_id\":\"first:approved::4\""));
    apply(r#"{"type":"ack"}"#);

    let wait_checkpoint = run(32);
    assert!(wait_checkpoint.contains("\"type\":\"persist_checkpoint\""));
    assert!(wait_checkpoint.contains("\"kind\":\"delta\""));
    assert!(wait_checkpoint.contains("\"base_revision\":1"));
    apply(r#"{"type":"persist_confirmed","revision":2}"#);

    let suspended = run(32);
    assert!(suspended.contains("\"type\":\"suspended\""));
    apply(r#"{"type":"event_payload","value":{"t":"string","v":"ok"}}"#);

    let after_event = run(32);
    assert!(after_event.contains("\"type\":\"persist_checkpoint\""));
    apply(r#"{"type":"persist_confirmed","revision":3}"#);

    let completed_persist = run(32);
    assert!(completed_persist.contains("\"type\":\"persist_checkpoint\""));
    apply(r#"{"type":"persist_confirmed","revision":4}"#);

    let done = run(32);
    assert!(done.contains("\"type\":\"completed\""));
    assert!(done.contains("\"t\":\"object\""));
    assert!(done.contains("\"approval\":{\"t\":\"string\",\"v\":\"ok\"}"));
    assert!(done.contains("\"result\":{\"t\":\"number\",\"v\":42}"));

    assert_eq!(tcc_continuation(), 0);
    let continuation = last_json();
    assert!(continuation.contains("\"status\":\"completed\""));
}

#[test]
fn c_abi_resume_does_not_replay_committed_prefix() {
    let artifact = encode_artifact(&Artifact::sdk_first_example("sdk-first")).unwrap();
    start(&artifact, "first");
    apply_effect_and_wait();
    assert_eq!(tcc_continuation(), 0);
    let snapshot = last_json();
    resume(&artifact, &snapshot);
    let outcome = run(32);
    assert!(outcome.contains("\"type\":\"suspended\""));
    assert!(!outcome.contains("\"type\":\"run_effect\""));
}

fn apply_effect_and_wait() {
    let first = run(32);
    assert!(first.contains("\"type\":\"run_effect\""));
    apply(r#"{"type":"effect_result","value":{"t":"number","v":42}}"#);
    let persist_effect = run(32);
    assert!(persist_effect.contains("\"type\":\"persist_effect\""));
    apply(r#"{"type":"ack"}"#);
    assert!(run(32).contains("\"type\":\"persist_checkpoint\""));
    apply(r#"{"type":"persist_confirmed","revision":1}"#);
    let wait = run(32);
    assert!(wait.contains("\"wait_id\":\"first:approved::4\""));
    apply(r#"{"type":"ack"}"#);
    assert!(run(32).contains("\"type\":\"persist_checkpoint\""));
    apply(r#"{"type":"persist_confirmed","revision":2}"#);
    assert!(run(32).contains("\"type\":\"suspended\""));
}

fn resume(artifact: &str, continuation: &str) {
    let artifact_buf = write_str(artifact);
    let cont_buf = write_str(continuation);
    let code = tcc_resume(artifact_buf.0, artifact_buf.1, cont_buf.0, cont_buf.1);
    tcc_free(artifact_buf.0, artifact_buf.1);
    tcc_free(cont_buf.0, cont_buf.1);
    assert_eq!(code, 0, "{}", last_json());
}

fn start_with_args(artifact: &str, execution_id: &str, args_json: &str) {
    let artifact_buf = write_str(artifact);
    let exec_buf = write_str(execution_id);
    let args_buf = write_str(args_json);
    let code = tcc_start_with_args(
        artifact_buf.0,
        artifact_buf.1,
        exec_buf.0,
        exec_buf.1,
        args_buf.0,
        args_buf.1,
    );
    tcc_free(artifact_buf.0, artifact_buf.1);
    tcc_free(exec_buf.0, exec_buf.1);
    tcc_free(args_buf.0, args_buf.1);
    assert_eq!(code, 0, "{}", last_json());
}

fn start(artifact: &str, execution_id: &str) {
    let artifact_buf = write_str(artifact);
    let exec_buf = write_str(execution_id);
    let code = tcc_start(artifact_buf.0, artifact_buf.1, exec_buf.0, exec_buf.1);
    tcc_free(artifact_buf.0, artifact_buf.1);
    tcc_free(exec_buf.0, exec_buf.1);
    assert_eq!(code, 0, "{}", last_json());
}

fn run(budget: u32) -> String {
    let code = tcc_run_until_host(budget);
    let json = last_json();
    assert_eq!(code, 0, "{json}");
    json
}

fn apply(json: &str) {
    let stamped = tcc_core::stamp_host_protocol_version(json).unwrap();
    let buf = write_str(&stamped);
    let code = tcc_apply_response(buf.0, buf.1);
    tcc_free(buf.0, buf.1);
    assert_eq!(code, 0, "{}", last_json());
}

fn write_str(text: &str) -> (*mut u8, u32) {
    let ptr = tcc_alloc(text.len() as u32);
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), ptr, text.len());
    }
    (ptr, text.len() as u32)
}

fn last_json() -> String {
    let len = tcc_json_len() as usize;
    let ptr = tcc_json_ptr();
    unsafe { String::from_utf8_lossy(slice::from_raw_parts(ptr, len)).into_owned() }
}

#[test]
fn c_abi_rejects_missing_host_protocol_version() {
    let artifact = encode_artifact(&Artifact::sdk_first_example("sdk-first")).unwrap();
    start(&artifact, "first");
    let first = run(32);
    assert!(first.contains("\"type\":\"run_effect\""));
    let buf = write_str(r#"{"type":"effect_result","value":{"t":"number","v":42}}"#);
    let code = tcc_apply_response(buf.0, buf.1);
    tcc_free(buf.0, buf.1);
    assert_ne!(code, 0);
    assert!(last_json().contains("host_protocol_version"));
}

#[test]
fn native_and_c_abi_agree_on_a_join() {
    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        Instruction::LoadConst {
            value: ConstValue::String("a".into()),
        },
        Instruction::Effect { has_input: false },
        Instruction::LoadConst {
            value: ConstValue::String("b".into()),
        },
        Instruction::Effect { has_input: false },
        Instruction::JoinAll,
        Instruction::Return,
    ];
    let spans = vec![None; instructions.len()];
    let artifact = Artifact {
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
                local_count: 0,
                param_defaults: Vec::new(),
                instructions,
                spans,
            }],
        },
    };
    let json = encode_artifact(&artifact).unwrap();
    let native_result = drive_native(artifact, "join-agree");
    start(&json, "join-agree");
    let cabi_result = drive_cabi();
    assert_eq!(native_result, cabi_result);
    assert!(cabi_result.contains("\"t\":\"array\""));
}

#[test]
fn native_and_c_abi_agree_on_a_race() {
    let instructions = vec![
        Instruction::Fork {
            count: 2,
            join_pc: Pc(5),
        },
        Instruction::LoadConst {
            value: ConstValue::String("a".into()),
        },
        Instruction::Effect { has_input: false },
        Instruction::LoadConst {
            value: ConstValue::String("b".into()),
        },
        Instruction::Effect { has_input: false },
        Instruction::JoinAny,
        Instruction::Return,
    ];
    let spans = vec![None; instructions.len()];
    let artifact = Artifact {
        envelope: Envelope {
            artifact_hash: "race".into(),
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
                local_count: 0,
                param_defaults: Vec::new(),
                instructions,
                spans,
            }],
        },
    };
    let json = encode_artifact(&artifact).unwrap();
    let native_result = drive_native(artifact, "race-agree");
    start(&json, "race-agree");
    let cabi_result = drive_cabi();
    assert_eq!(native_result, cabi_result);
    assert!(cabi_result.contains("\"t\":\"number\""));
    assert!(!cabi_result.contains("\"t\":\"array\""));
}

#[test]
fn generated_native_matches_c_abi() {
    for seed in 0..seed_count().min(16) {
        let artifact = generate(seed);
        let json = encode_artifact(&artifact).unwrap();
        let native_result = drive_native(artifact, &format!("gen-{seed}"));
        start(&json, &format!("gen-{seed}"));
        let cabi_result = drive_cabi();
        assert_eq!(native_result, cabi_result, "seed {seed}");
    }
}

#[test]
fn native_and_c_abi_agree_on_program_input() {
    let instructions = vec![
        Instruction::LoadLocal { local: LocalId(0) },
        Instruction::Return,
    ];
    let spans = vec![None; instructions.len()];
    let artifact = Artifact {
        envelope: Envelope {
            artifact_hash: "program-input".into(),
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
                param_count: 1,
                local_count: 1,
                param_defaults: Vec::new(),
                instructions,
                spans,
            }],
        },
    };
    let json = encode_artifact(&artifact).unwrap();
    let mut native = NativeEngine::start_with_args(
        artifact,
        "program-input",
        &EngineCaps::current(),
        &[Value::String("hi".into())],
    )
    .unwrap();
    let native_result = loop {
        match native.run_until_host(32) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                native
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Completed { .. } => {
                break String::from_utf8(
                    tcc_state::encode_continuation(native.continuation()).unwrap(),
                )
                .unwrap();
            }
            other => panic!("native {other:?}"),
        }
    };
    start_with_args(&json, "program-input", r#"[{"t":"string","v":"hi"}]"#);
    let cabi_result = drive_cabi();
    assert_eq!(native_result, cabi_result);
    assert!(cabi_result.contains("\"v\":\"hi\""));
}

fn drive_native(artifact: Artifact, execution_id: &str) -> String {
    let mut engine = NativeEngine::start(artifact, execution_id, &EngineCaps::current()).unwrap();
    loop {
        match engine.run_until_host(256) {
            EngineOutcome::Host(HostRequest::RunEffect { key, .. }) => {
                let value = if key == "skipped" {
                    Value::Number(99.0)
                } else {
                    Value::Number(42.0)
                };
                engine
                    .apply_host_response(HostResponse::EffectResult { value })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistEffect { .. })
            | EngineOutcome::Host(HostRequest::RegisterWait { .. })
            | EngineOutcome::Host(HostRequest::RegisterTimer { .. })
            | EngineOutcome::Host(HostRequest::CreateChild { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Suspended => match &engine.continuation().pending {
                Some(tcc_state::PendingOp::Wait {
                    kind: tcc_state::WaitKind::Timer { .. },
                }) => engine
                    .apply_host_response(HostResponse::TimerFired { branch: None })
                    .unwrap(),
                Some(tcc_state::PendingOp::Wait {
                    kind: tcc_state::WaitKind::Child { .. },
                }) => engine
                    .apply_host_response(HostResponse::ChildResult {
                        value: Value::Number(7.0),
                        branch: None,
                    })
                    .unwrap(),
                _ => engine
                    .apply_host_response(HostResponse::EventPayload {
                        value: Value::String("ok".into()),
                        branch: None,
                    })
                    .unwrap(),
            },
            EngineOutcome::Completed { .. } => {
                return String::from_utf8(
                    tcc_state::encode_continuation(engine.continuation()).unwrap(),
                )
                .unwrap();
            }
            other => panic!("native {other:?}"),
        }
    }
}

fn drive_cabi() -> String {
    loop {
        let outcome = run(256);
        if outcome.contains("\"type\":\"completed\"") {
            return continuation_json();
        }
        if outcome.contains("\"type\":\"run_effect\"") {
            if outcome.contains("\"key\":\"skipped\"") {
                apply(r#"{"type":"effect_result","value":{"t":"number","v":99}}"#);
            } else {
                apply(r#"{"type":"effect_result","value":{"t":"number","v":42}}"#);
            }
        } else if outcome.contains("\"type\":\"persist_checkpoint\"") {
            let revision = persist_revision(&outcome);
            apply(&format!(
                r#"{{"type":"persist_confirmed","revision":{revision}}}"#
            ));
        } else if outcome.contains("\"type\":\"suspended\"") {
            let continuation = continuation_json();
            if continuation.contains("\"timer\"") {
                apply(r#"{"type":"timer_fired"}"#);
            } else if continuation.contains("\"child\"") {
                apply(r#"{"type":"child_result","value":{"t":"number","v":7}}"#);
            } else {
                apply(r#"{"type":"event_payload","value":{"t":"string","v":"ok"}}"#);
            }
        } else {
            apply(r#"{"type":"ack"}"#);
        }
    }
}

fn persist_revision(outcome: &str) -> u64 {
    let key = "\"revision\":";
    let start = outcome.find(key).expect("revision") + key.len();
    outcome[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap()
}

fn continuation_json() -> String {
    let code = tcc_continuation();
    let json = last_json();
    assert_eq!(code, 0, "{json}");
    json
}
