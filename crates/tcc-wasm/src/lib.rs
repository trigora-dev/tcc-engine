// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! WASM exports for the TCC core.
//!
//! JSON crosses the boundary through linear memory. The module does not use
//! WASI networking, filesystem, or threads.

use std::cell::RefCell;
use std::slice;

use tcc_core::{decode_args_array, decode_response, encode_outcome, Engine};
use tcc_ir::{decode_artifact, EngineCaps, ENGINE_FORMAT_VERSION};
use tcc_state::{decode_continuation, encode_continuation, Value};

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
    match start_engine(artifact_json, execution_id, &[]) {
        Ok(()) => 0,
        Err(message) => set_error(&message),
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn tcc_start_with_args(
    artifact_ptr: *const u8,
    artifact_len: u32,
    exec_ptr: *const u8,
    exec_len: u32,
    args_ptr: *const u8,
    args_len: u32,
) -> i32 {
    let artifact_json = unsafe { read_str(artifact_ptr, artifact_len) };
    let execution_id = unsafe { read_str(exec_ptr, exec_len) };
    let args_json = unsafe { read_str(args_ptr, args_len) };
    let args = match decode_args_array(args_json) {
        Ok(args) => args,
        Err(err) => return set_error(&err.to_string()),
    };
    match start_engine(artifact_json, execution_id, &args) {
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

fn start_engine(artifact_json: &str, execution_id: &str, args: &[Value]) -> Result<(), String> {
    let artifact = decode_artifact(artifact_json).map_err(|err| err.to_string())?;
    let engine = Engine::start_with_args(artifact, execution_id, &EngineCaps::current(), args)
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
mod tests;
