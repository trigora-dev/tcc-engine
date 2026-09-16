//! Execution core: run a validated artifact until the host must participate.

#![allow(clippy::derive_partial_eq_without_eq)]

pub mod engine;
pub mod error;
pub mod protocol;

pub use engine::{Engine, EngineOutcome};
pub use error::CoreError;
pub use protocol::{
    ChildSpec, EffectRecord, EffectStatus, HostRequest, HostResponse, WaitRegistration,
};
