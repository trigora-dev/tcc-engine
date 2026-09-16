//! Values, frames, and continuation encoding.
//!
//! Persist explicit program state. Do not dump Rust layouts, native pointers,
//! or WASM memory.

#![allow(clippy::derive_partial_eq_without_eq)]

pub mod continuation;
pub mod encode;
pub mod error;
mod json;
pub mod value;

pub use continuation::{ArtifactId, Continuation, ContinuationStatus, Frame, PendingOp, WaitKind};
pub use encode::{decode_continuation, decode_value, encode_continuation, encode_value};
pub use error::StateError;
pub use value::Value;
