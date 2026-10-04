// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use std::time::Duration;

/// What the host just finished. The callback runs after the timed region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostTraceKind {
    CheckpointPersisted,
    Effect,
    ContinuationRestored,
}

/// One diagnostic sample. Continuation `body` is set only when the host was asked to retain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTraceEvent {
    pub kind: HostTraceKind,
    pub revision: Option<u64>,
    pub duration: Duration,
    pub bytes: Option<usize>,
    pub body: Option<String>,
    pub effect_key: Option<String>,
    pub journal_hit: bool,
    pub suspended: bool,
}

pub type HostTrace = Box<dyn FnMut(HostTraceEvent) + Send>;
