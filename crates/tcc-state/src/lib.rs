// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! Values, frames, and continuation encoding.
//!
//! Persist explicit program state. Do not dump Rust layouts, native pointers,
//! or WASM memory.

#![allow(clippy::derive_partial_eq_without_eq)]

pub mod continuation;
pub mod delta;
pub mod encode;
pub mod error;
pub mod heap;
pub mod json;
pub mod value;

pub use continuation::{
    ArtifactId, BranchOp, BranchPhase, Continuation, ContinuationStatus, Frame, JoinBranch,
    JoinKind, JoinReentry, JoinState, JoinStatus, PendingOp, TryHandler, WaitKind,
};
pub use delta::{
    apply_continuation_delta, continuation_delta_to_json, decode_continuation_delta,
    diff_continuation, encode_continuation_delta, json_to_continuation_delta, persist_intent,
    ContinuationDelta, FrameDelta, LocalPatch, PersistIntent, PersistKind, MATERIALIZE_EVERY,
};
pub use encode::{decode_continuation, decode_value, encode_continuation, encode_value};
pub use error::StateError;
pub use heap::{absorb_continuation, absorb_value, export_value, gc_heap, structural_eq};
pub use value::{numbers_durable_eq, numbers_strict_eq, HeapCell, Value};
