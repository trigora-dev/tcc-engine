// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! Continuation-local collections.
//!
//! Locals, the operand stack, and cells hold [`Value::Ref`]. Assignment copies
//! the ref. Mutation updates the cell. Detached `Object` / `Array` values exist
//! only on the host boundary and are absorbed before they become continuation state.

use std::collections::BTreeMap;

use crate::continuation::{Continuation, JoinState};
use crate::error::StateError;
use crate::value::{HeapCell, Value};

pub fn absorb_value(heap: &mut Vec<HeapCell>, value: Value) -> Value {
    match value {
        Value::Object(fields) => {
            let id = heap.len() as u32;
            heap.push(HeapCell::Hole);
            let mut mapped = BTreeMap::new();
            for (key, child) in fields {
                mapped.insert(key, absorb_value(heap, child));
            }
            heap[id as usize] = HeapCell::Object(mapped);
            Value::Ref(id)
        }
        Value::Array(items) => {
            let id = heap.len() as u32;
            heap.push(HeapCell::Hole);
            let mapped = items
                .into_iter()
                .map(|item| absorb_value(heap, item))
                .collect();
            heap[id as usize] = HeapCell::Array(mapped);
            Value::Ref(id)
        }
        other => other,
    }
}

pub fn absorb_continuation(continuation: &mut Continuation) {
    for frame in &mut continuation.frames {
        for local in &mut frame.locals {
            *local = absorb_value(
                &mut continuation.heap,
                std::mem::replace(local, Value::Undefined),
            );
        }
    }
    for value in &mut continuation.stack {
        *value = absorb_value(
            &mut continuation.heap,
            std::mem::replace(value, Value::Undefined),
        );
    }
    if let Some(result) = continuation.result.take() {
        continuation.result = Some(absorb_value(&mut continuation.heap, result));
    }
    if let Some(join) = continuation.join.as_mut() {
        absorb_join(&mut continuation.heap, join);
    }
    for cell_index in 0..continuation.heap.len() {
        absorb_cell(&mut continuation.heap, cell_index);
    }
}

fn absorb_join(heap: &mut Vec<HeapCell>, join: &mut JoinState) {
    for branch in &mut join.branches {
        if let Some(result) = branch.result.take() {
            branch.result = Some(absorb_value(heap, result));
        }
    }
}

fn absorb_cell(heap: &mut Vec<HeapCell>, index: usize) {
    let cell = std::mem::replace(&mut heap[index], HeapCell::Hole);
    let cell = match cell {
        HeapCell::Object(fields) => {
            let mut mapped = BTreeMap::new();
            for (key, value) in fields {
                mapped.insert(key, absorb_value(heap, value));
            }
            HeapCell::Object(mapped)
        }
        HeapCell::Array(items) => HeapCell::Array(
            items
                .into_iter()
                .map(|item| absorb_value(heap, item))
                .collect(),
        ),
        HeapCell::Cell(value) => HeapCell::Cell(absorb_value(heap, value)),
        HeapCell::Env(cells) => HeapCell::Env(cells),
        HeapCell::Closure { func, env } => HeapCell::Closure { func, env },
        HeapCell::Hole => HeapCell::Hole,
    };
    heap[index] = cell;
}

/// Inline a ref for a host payload. Cycles cannot be inlined.
pub fn export_value(heap: &[HeapCell], value: &Value) -> Result<Value, StateError> {
    let mut seen = Vec::new();
    export_seen(heap, value, &mut seen)
}

fn export_seen(heap: &[HeapCell], value: &Value, seen: &mut Vec<u32>) -> Result<Value, StateError> {
    match value {
        Value::Ref(id) => {
            if seen.contains(id) {
                return Err(StateError::InvalidEncoding(
                    "cyclic collection cannot cross a host boundary".to_string(),
                ));
            }
            seen.push(*id);
            let exported = match heap.get(*id as usize) {
                Some(HeapCell::Object(fields)) => {
                    let mut mapped = BTreeMap::new();
                    for (key, child) in fields {
                        mapped.insert(key.clone(), export_seen(heap, child, seen)?);
                    }
                    Value::Object(mapped)
                }
                Some(HeapCell::Array(items)) => {
                    let mut mapped = Vec::with_capacity(items.len());
                    for item in items {
                        mapped.push(export_seen(heap, item, seen)?);
                    }
                    Value::Array(mapped)
                }
                Some(HeapCell::Closure { .. }) => {
                    return Err(StateError::InvalidEncoding(
                        "a closure cannot cross a host boundary".to_string(),
                    ));
                }
                Some(HeapCell::Cell(_) | HeapCell::Env(_)) => {
                    return Err(StateError::InvalidEncoding(
                        "a captured binding cannot cross a host boundary".to_string(),
                    ));
                }
                _ => {
                    return Err(StateError::InvalidEncoding(format!(
                        "dangling collection ref {id}"
                    )))
                }
            };
            seen.pop();
            Ok(exported)
        }
        Value::Object(fields) => {
            let mut mapped = BTreeMap::new();
            for (key, child) in fields {
                mapped.insert(key.clone(), export_seen(heap, child, seen)?);
            }
            Ok(Value::Object(mapped))
        }
        Value::Array(items) => {
            let mut mapped = Vec::with_capacity(items.len());
            for item in items {
                mapped.push(export_seen(heap, item, seen)?);
            }
            Ok(Value::Array(mapped))
        }
        other => Ok(other.clone()),
    }
}

pub fn gc_heap(continuation: &mut Continuation) {
    let len = continuation.heap.len();
    if len == 0 {
        return;
    }
    let mut seen = vec![false; len];
    for frame in &continuation.frames {
        for local in &frame.locals {
            mark(&continuation.heap, local, &mut seen);
        }
    }
    for value in &continuation.stack {
        mark(&continuation.heap, value, &mut seen);
    }
    if let Some(result) = &continuation.result {
        mark(&continuation.heap, result, &mut seen);
    }
    if let Some(join) = &continuation.join {
        for branch in &join.branches {
            if let Some(result) = &branch.result {
                mark(&continuation.heap, result, &mut seen);
            }
        }
    }
    for id in &continuation.iterating {
        if let Some(slot) = seen.get_mut(*id as usize) {
            *slot = true;
        }
    }
    for id in continuation.func_refs.iter().flatten() {
        mark(&continuation.heap, &Value::Ref(*id), &mut seen);
    }
    for (index, live) in seen.iter().enumerate() {
        if !live {
            continuation.heap[index] = HeapCell::Hole;
        }
    }
    while matches!(continuation.heap.last(), Some(HeapCell::Hole)) {
        continuation.heap.pop();
    }
}

fn mark(heap: &[HeapCell], value: &Value, seen: &mut [bool]) {
    let Value::Ref(id) = value else {
        return;
    };
    let index = *id as usize;
    if seen.get(index).copied().unwrap_or(true) {
        return;
    }
    seen[index] = true;
    match heap.get(index) {
        Some(HeapCell::Object(fields)) => {
            for child in fields.values() {
                mark(heap, child, seen);
            }
        }
        Some(HeapCell::Array(items)) => {
            for child in items {
                mark(heap, child, seen);
            }
        }
        Some(HeapCell::Cell(value)) => mark(heap, value, seen),
        Some(HeapCell::Env(cells)) => {
            for id in cells {
                mark(heap, &Value::Ref(*id), seen);
            }
        }
        Some(HeapCell::Closure { env, .. }) => mark(heap, &Value::Ref(*env), seen),
        _ => {}
    }
}

/// Python `==` for collections. A cycle in the walk is an error.
pub fn structural_eq(heap: &[HeapCell], left: &Value, right: &Value) -> Result<bool, StateError> {
    let mut seen = Vec::new();
    structural_eq_seen(heap, left, right, &mut seen)
}

fn structural_eq_seen(
    heap: &[HeapCell],
    left: &Value,
    right: &Value,
    seen: &mut Vec<(u32, u32)>,
) -> Result<bool, StateError> {
    match (left, right) {
        (Value::Ref(left_id), Value::Ref(right_id)) => {
            if left_id == right_id {
                return Ok(true);
            }
            let pair = (*left_id, *right_id);
            if seen.contains(&pair) {
                return Err(StateError::InvalidEncoding(
                    "cyclic collection comparison".to_string(),
                ));
            }
            seen.push(pair);
            let equal = match (heap.get(*left_id as usize), heap.get(*right_id as usize)) {
                (Some(HeapCell::Array(left_items)), Some(HeapCell::Array(right_items))) => {
                    if left_items.len() != right_items.len() {
                        false
                    } else {
                        for (left_item, right_item) in left_items.iter().zip(right_items) {
                            if !structural_eq_seen(heap, left_item, right_item, seen)? {
                                seen.pop();
                                return Ok(false);
                            }
                        }
                        true
                    }
                }
                (Some(HeapCell::Object(left_fields)), Some(HeapCell::Object(right_fields))) => {
                    if left_fields.len() != right_fields.len() {
                        false
                    } else {
                        for (key, left_value) in left_fields {
                            let Some(right_value) = right_fields.get(key) else {
                                seen.pop();
                                return Ok(false);
                            };
                            if !structural_eq_seen(heap, left_value, right_value, seen)? {
                                seen.pop();
                                return Ok(false);
                            }
                        }
                        true
                    }
                }
                _ => false,
            };
            seen.pop();
            Ok(equal)
        }
        (Value::Number(left_n), Value::Number(right_n)) => Ok(left_n == right_n),
        (Value::Bool(left_b), Value::Bool(right_b)) => Ok(left_b == right_b),
        (Value::Bool(flag), Value::Number(number)) | (Value::Number(number), Value::Bool(flag)) => {
            Ok((*number == 1.0 && *flag) || (*number == 0.0 && !*flag))
        }
        (Value::String(left_s), Value::String(right_s)) => Ok(left_s == right_s),
        (Value::Null, Value::Null) | (Value::Undefined, Value::Undefined) => Ok(true),
        (Value::Object(left_fields), Value::Object(right_fields)) => {
            if left_fields.len() != right_fields.len() {
                return Ok(false);
            }
            for (key, left_value) in left_fields {
                let Some(right_value) = right_fields.get(key) else {
                    return Ok(false);
                };
                if !structural_eq_seen(heap, left_value, right_value, seen)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Array(left_items), Value::Array(right_items)) => {
            if left_items.len() != right_items.len() {
                return Ok(false);
            }
            for (left_item, right_item) in left_items.iter().zip(right_items) {
                if !structural_eq_seen(heap, left_item, right_item, seen)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}
