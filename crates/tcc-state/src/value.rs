// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use std::collections::BTreeMap;

/// TypeScript-subset runtime values. JavaScript `number` is IEEE-754 f64.
/// Later languages must not silently reuse this for Python integers.
#[derive(Debug, Clone)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// Detached host payload. Continuations absorb these into [`HeapCell`]s.
    Object(BTreeMap<String, Value>),
    Array(Vec<Value>),
    /// Identity of an object or array cell in the continuation heap.
    Ref(u32),
}

/// Continuation-local collection. Slots hold [`Value::Ref`], not a copy of the cell.
#[derive(Debug, Clone, PartialEq)]
pub enum HeapCell {
    Object(BTreeMap<String, Value>),
    Array(Vec<Value>),
    /// One captured binding. The defining activation and every closure share this cell.
    Cell(Value),
    /// Refs to [`HeapCell::Cell`] values, not copies of those values.
    Env(Vec<u32>),
    /// A function plus the environment it closes over.
    Closure {
        func: u32,
        env: u32,
    },
    /// Unreachable id kept stable so later deltas name the same survivors.
    Hole,
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
            (Value::Bool(left), Value::Bool(right)) => left == right,
            (Value::Number(left), Value::Number(right)) => numbers_durable_eq(*left, *right),
            (Value::String(left), Value::String(right)) => left == right,
            (Value::Object(left), Value::Object(right)) => left == right,
            (Value::Array(left), Value::Array(right)) => left == right,
            (Value::Ref(left), Value::Ref(right)) => left == right,
            _ => false,
        }
    }
}

/// Checkpoint equality. Canonical NaNs match. `-0` does not match `+0`.
pub fn numbers_durable_eq(left: f64, right: f64) -> bool {
    if left.is_nan() && right.is_nan() {
        true
    } else {
        left.to_bits() == right.to_bits()
    }
}

/// JavaScript `===` for numbers: `NaN` is not equal to itself, and `-0` equals `+0`.
pub fn numbers_strict_eq(left: f64, right: f64) -> bool {
    left == right
}
