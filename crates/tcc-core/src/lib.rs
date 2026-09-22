//! Execution core: run a validated artifact until the host must participate.

#![allow(clippy::derive_partial_eq_without_eq)]

pub mod engine;
pub mod error;
pub mod protocol;
pub mod wire;

pub use engine::{Engine, EngineOutcome};
pub use error::CoreError;
pub use protocol::{
    ChildSpec, EffectRecord, EffectStatus, HostRequest, HostResponse, WaitRegistration,
    HOST_PROTOCOL_VERSION,
};
pub use wire::{
    decode_args_array, decode_request, decode_response, encode_outcome, encode_request,
    stamp_host_protocol_version,
};
