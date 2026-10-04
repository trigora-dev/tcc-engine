// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! SQLite host. It commits continuation snapshots and durable rows.
//! The caller injects effects and decides when a ready child or due timer runs.

mod crash;
mod host;
mod runtime;
mod store;
mod trace;

pub use crash::Crash;
pub use host::{EffectProvider, SqliteHost};
pub use runtime::{
    read_continuation, resume_execution, start_execution, start_execution_with_args, RunResult,
};
pub use store::{ChildRow, EffectRow, Snapshot, Store, TimerRow, WaitRow};
pub use trace::{HostTrace, HostTraceEvent, HostTraceKind};

#[cfg(test)]
mod windows;
