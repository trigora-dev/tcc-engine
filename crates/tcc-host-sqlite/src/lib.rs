//! SQLite host. It commits continuation snapshots and durable rows.
//! The caller injects effects and decides when a ready child or due timer runs.

mod crash;
mod host;
mod runtime;
mod store;

pub use crash::Crash;
pub use host::SqliteHost;
pub use runtime::{read_continuation, resume_execution, start_execution, RunResult};
pub use store::{ChildRow, EffectRow, Snapshot, Store, TimerRow, WaitRow};

#[cfg(test)]
mod windows;
