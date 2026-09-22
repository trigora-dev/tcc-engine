//! WASM exports for the TCC core.
//!
//! JSON crosses the boundary through linear memory. The module does not use
//! WASI networking, filesystem, or threads.

use std::cell::RefCell;
use std::slice;

use tcc_core::{decode_response, encode_outcome, Engine};
use tcc_ir::{decode_artifact, EngineCaps, ENGINE_FORMAT_VERSION};
use tcc_state::{decode_continuation, encode_continuation};

pub use tcc_core::{
    ChildSpec, EffectRecord, EffectStatus, Engine as CoreEngine, EngineOutcome, HostRequest,
    HostResponse, WaitRegistration,
};
pub use tcc_ir::{validate, Artifact};

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static LAST: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Stable numeric export for host bindings that load the module.
#[no_mangle]
pub extern "C" fn tcc_engine_format_version() -> u32 {
    ENGINE_FORMAT_VERSION
}

#[no_mangle]
pub extern "C" fn tcc_alloc(len: u32) -> *mut u8 {
    let mut buf = vec![0u8; len as usize];
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn tcc_free(ptr: *mut u8, len: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = Vec::from_raw_parts(ptr, len as usize, len as usize);
    }
}

#[no_mangle]
pub extern "C" fn tcc_json_ptr() -> *const u8 {
    LAST.with(|last| last.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn tcc_json_len() -> u32 {
    LAST.with(|last| last.borrow().len() as u32)
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn tcc_start(
    artifact_ptr: *const u8,
    artifact_len: u32,
    exec_ptr: *const u8,
    exec_len: u32,
) -> i32 {
    let artifact_json = unsafe { read_str(artifact_ptr, artifact_len) };
    let execution_id = unsafe { read_str(exec_ptr, exec_len) };
    match start_engine(artifact_json, execution_id) {
        Ok(()) => 0,
        Err(message) => set_error(&message),
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn tcc_resume(
    artifact_ptr: *const u8,
    artifact_len: u32,
    continuation_ptr: *const u8,
    continuation_len: u32,
) -> i32 {
    let artifact_json = unsafe { read_str(artifact_ptr, artifact_len) };
    let continuation_json = unsafe { read_str(continuation_ptr, continuation_len) };
    match resume_engine(artifact_json, continuation_json) {
        Ok(()) => 0,
        Err(message) => set_error(&message),
    }
}

#[no_mangle]
pub extern "C" fn tcc_run_until_host(budget: u32) -> i32 {
    ENGINE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let engine = match slot.as_mut() {
            Some(engine) => engine,
            None => return set_error("engine not started"),
        };
        match encode_outcome(&engine.run_until_host(budget)) {
            Ok(json) => set_ok(json.into_bytes()),
            Err(err) => set_error(&err.to_string()),
        }
    })
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn tcc_apply_response(ptr: *const u8, len: u32) -> i32 {
    let json = unsafe { read_str(ptr, len) };
    ENGINE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let engine = match slot.as_mut() {
            Some(engine) => engine,
            None => return set_error("engine not started"),
        };
        match decode_response(json) {
            Ok(response) => match engine.apply_host_response(response) {
                Ok(()) => 0,
                Err(err) => set_error(&err.to_string()),
            },
            Err(err) => set_error(&err.to_string()),
        }
    })
}

#[no_mangle]
pub extern "C" fn tcc_continuation() -> i32 {
    ENGINE.with(|slot| {
        let slot = slot.borrow();
        let engine = match slot.as_ref() {
            Some(engine) => engine,
            None => return set_error("engine not started"),
        };
        match encode_continuation(engine.continuation()) {
            Ok(bytes) => set_ok(bytes),
            Err(err) => set_error(&err.to_string()),
        }
    })
}

fn start_engine(artifact_json: &str, execution_id: &str) -> Result<(), String> {
    let artifact = decode_artifact(artifact_json).map_err(|err| err.to_string())?;
    let engine = Engine::start(artifact, execution_id, &EngineCaps::current())
        .map_err(|err| err.to_string())?;
    ENGINE.with(|slot| {
        *slot.borrow_mut() = Some(engine);
    });
    Ok(())
}

fn resume_engine(artifact_json: &str, continuation_json: &str) -> Result<(), String> {
    let artifact = decode_artifact(artifact_json).map_err(|err| err.to_string())?;
    let continuation =
        decode_continuation(continuation_json.as_bytes()).map_err(|err| err.to_string())?;
    let engine = Engine::resume(artifact, continuation, &EngineCaps::current())
        .map_err(|err| err.to_string())?;
    ENGINE.with(|slot| {
        *slot.borrow_mut() = Some(engine);
    });
    Ok(())
}

unsafe fn read_str<'a>(ptr: *const u8, len: u32) -> &'a str {
    let bytes = slice::from_raw_parts(ptr, len as usize);
    std::str::from_utf8(bytes).unwrap_or("")
}

fn set_ok(bytes: Vec<u8>) -> i32 {
    LAST.with(|last| *last.borrow_mut() = bytes);
    0
}

fn set_error(message: &str) -> i32 {
    LAST.with(|last| *last.borrow_mut() = message.as_bytes().to_vec());
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use tcc_core::{Engine as NativeEngine, EngineOutcome, HostRequest, HostResponse};
    use tcc_ir::encode_artifact;
    use tcc_ir::gen::{generate, seed_count};
    use tcc_ir::{ConstValue, Envelope, FuncId, Function, Instruction, Pc, Program};
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
            Instruction::Effect,
            Instruction::LoadConst {
                value: ConstValue::String("b".into()),
            },
            Instruction::Effect,
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
            Instruction::Effect,
            Instruction::LoadConst {
                value: ConstValue::String("b".into()),
            },
            Instruction::Effect,
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

    fn drive_native(artifact: Artifact, execution_id: &str) -> String {
        let mut engine =
            NativeEngine::start(artifact, execution_id, &EngineCaps::current()).unwrap();
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
}
