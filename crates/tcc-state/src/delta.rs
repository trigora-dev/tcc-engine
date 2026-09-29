// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! Semantic continuation deltas used as persist intent.
//!
//! Deltas name changed continuation fields; they are not a byte-diff of JSON.
//! Reconstruction is host packing and must yield a continuation in this crate's
//! format. Resume still consumes a full continuation.

use std::collections::BTreeMap;

use crate::continuation::{
    Continuation, ContinuationStatus, JoinReentry, JoinState, PendingOp, TryHandler,
};
use crate::encode::{
    cell_to_json, join_to_json, json_to_cell, json_to_join, json_to_pending, json_to_try_handler,
    json_to_value, parse_status, pending_to_json, status_name, try_handler_to_json, value_to_json,
};
use crate::error::StateError;
use crate::heap::absorb_continuation;
use crate::json::Json;
use crate::value::{HeapCell, Value};

/// Force a snapshot at least every `N` deltas so recovery walks a bounded suffix.
pub const MATERIALIZE_EVERY: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistKind {
    Snapshot,
    Delta,
}

impl PersistKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PersistKind::Snapshot => "snapshot",
            PersistKind::Delta => "delta",
        }
    }

    pub fn parse(name: &str) -> Result<Self, StateError> {
        match name {
            "snapshot" => Ok(PersistKind::Snapshot),
            "delta" => Ok(PersistKind::Delta),
            _ => Err(StateError::InvalidEncoding(format!(
                "unknown persist kind `{name}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalPatch {
    pub slot: u32,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameDelta {
    pub index: u32,
    pub pc: Option<u32>,
    pub locals: Vec<LocalPatch>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContinuationDelta {
    pub frames: Vec<FrameDelta>,
    pub stack: Option<Vec<Value>>,
    pub pending: Option<Option<PendingOp>>,
    pub status: Option<ContinuationStatus>,
    pub result: Option<Option<Value>>,
    pub try_stack: Option<Vec<TryHandler>>,
    pub join: Option<Option<JoinState>>,
    pub reentries: Option<Vec<JoinReentry>>,
    /// Full heap when any cell changed. Ids stay stable across the replacement.
    pub heap: Option<Vec<HeapCell>>,
    pub iterating: Option<Vec<u32>>,
    pub func_refs: Option<Vec<Option<u32>>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PersistIntent {
    pub kind: PersistKind,
    pub materialize: bool,
    pub delta: Option<ContinuationDelta>,
}

pub fn persist_intent(
    confirmed: Option<&Continuation>,
    current: &Continuation,
    deltas_since_snapshot: u32,
) -> PersistIntent {
    let Some(base) = confirmed else {
        return snapshot_intent();
    };
    if deltas_since_snapshot >= MATERIALIZE_EVERY {
        return snapshot_intent();
    }
    match diff_continuation(base, current) {
        None => snapshot_intent(),
        Some(delta) if !delta_cheaper_than_snapshot(&delta, current) => snapshot_intent(),
        Some(delta) => PersistIntent {
            kind: PersistKind::Delta,
            materialize: false,
            delta: Some(delta),
        },
    }
}

fn snapshot_intent() -> PersistIntent {
    PersistIntent {
        kind: PersistKind::Snapshot,
        materialize: true,
        delta: None,
    }
}

/// Structural diff against the last confirmed continuation.
///
/// Returns `None` when frame topology changed and a snapshot is required.
pub fn diff_continuation(base: &Continuation, current: &Continuation) -> Option<ContinuationDelta> {
    if base.frames.len() != current.frames.len() {
        return None;
    }
    let mut frames = Vec::new();
    for (index, (before, after)) in base.frames.iter().zip(current.frames.iter()).enumerate() {
        if before.func_id != after.func_id || before.locals.len() != after.locals.len() {
            return None;
        }
        let mut locals = Vec::new();
        for (slot, (left, right)) in before.locals.iter().zip(after.locals.iter()).enumerate() {
            if left != right {
                locals.push(LocalPatch {
                    slot: slot as u32,
                    value: right.clone(),
                });
            }
        }
        let pc = if before.pc == after.pc {
            None
        } else {
            Some(after.pc)
        };
        if pc.is_some() || !locals.is_empty() {
            frames.push(FrameDelta {
                index: index as u32,
                pc,
                locals,
            });
        }
    }
    Some(ContinuationDelta {
        frames,
        stack: if base.stack == current.stack {
            None
        } else {
            Some(current.stack.clone())
        },
        pending: if base.pending == current.pending {
            None
        } else {
            Some(current.pending.clone())
        },
        status: if base.status == current.status {
            None
        } else {
            Some(current.status)
        },
        result: if base.result == current.result {
            None
        } else {
            Some(current.result.clone())
        },
        try_stack: if base.try_stack == current.try_stack {
            None
        } else {
            Some(current.try_stack.clone())
        },
        join: if base.join == current.join {
            None
        } else {
            Some(current.join.clone())
        },
        reentries: if base.reentries == current.reentries {
            None
        } else {
            Some(current.reentries.clone())
        },
        heap: if base.heap == current.heap {
            None
        } else {
            Some(current.heap.clone())
        },
        iterating: if base.iterating == current.iterating {
            None
        } else {
            Some(current.iterating.clone())
        },
        func_refs: if base.func_refs == current.func_refs {
            None
        } else {
            Some(current.func_refs.clone())
        },
    })
}

pub fn apply_continuation_delta(
    mut base: Continuation,
    delta: &ContinuationDelta,
) -> Result<Continuation, StateError> {
    for frame_delta in &delta.frames {
        let frame = base
            .frames
            .get_mut(frame_delta.index as usize)
            .ok_or_else(|| {
                StateError::InvalidEncoding(format!(
                    "delta frame index {} is out of range",
                    frame_delta.index
                ))
            })?;
        if let Some(pc) = frame_delta.pc {
            frame.pc = pc;
        }
        for patch in &frame_delta.locals {
            let slot = frame.locals.get_mut(patch.slot as usize).ok_or_else(|| {
                StateError::InvalidEncoding(format!(
                    "delta local slot {} is out of range",
                    patch.slot
                ))
            })?;
            *slot = patch.value.clone();
        }
    }
    if let Some(stack) = &delta.stack {
        base.stack = stack.clone();
    }
    if let Some(pending) = &delta.pending {
        base.pending = pending.clone();
    }
    if let Some(status) = delta.status {
        base.status = status;
    }
    if let Some(result) = &delta.result {
        base.result = result.clone();
    }
    if let Some(try_stack) = &delta.try_stack {
        base.try_stack = try_stack.clone();
    }
    if let Some(join) = &delta.join {
        base.join = join.clone();
    }
    if let Some(reentries) = &delta.reentries {
        base.reentries = reentries.clone();
    }
    if let Some(heap) = &delta.heap {
        base.heap = heap.clone();
    }
    if let Some(iterating) = &delta.iterating {
        base.iterating = iterating.clone();
    }
    if let Some(func_refs) = &delta.func_refs {
        base.func_refs = func_refs.clone();
    }
    absorb_continuation(&mut base);
    Ok(base)
}

pub fn continuation_delta_to_json(delta: &ContinuationDelta) -> Json {
    let mut map = BTreeMap::new();
    if !delta.frames.is_empty() {
        map.insert(
            "frames".to_string(),
            Json::Array(delta.frames.iter().map(frame_delta_to_json).collect()),
        );
    }
    if let Some(stack) = &delta.stack {
        map.insert(
            "stack".to_string(),
            Json::Array(stack.iter().map(value_to_json).collect()),
        );
    }
    if let Some(pending) = &delta.pending {
        map.insert(
            "pending".to_string(),
            match pending {
                Some(value) => pending_to_json(value),
                None => Json::Null,
            },
        );
    }
    if let Some(status) = delta.status {
        map.insert(
            "status".to_string(),
            Json::String(status_name(status).to_string()),
        );
    }
    if let Some(result) = &delta.result {
        map.insert(
            "result".to_string(),
            match result {
                Some(value) => value_to_json(value),
                None => Json::Null,
            },
        );
    }
    if let Some(try_stack) = &delta.try_stack {
        map.insert(
            "try_stack".to_string(),
            Json::Array(try_stack.iter().map(try_handler_to_json).collect()),
        );
    }
    if let Some(join) = &delta.join {
        map.insert(
            "join".to_string(),
            match join {
                Some(value) => join_to_json(value),
                None => Json::Null,
            },
        );
    }
    if let Some(reentries) = &delta.reentries {
        map.insert(
            "reentries".to_string(),
            Json::Array(reentries.iter().map(reentry_delta_to_json).collect()),
        );
    }
    if let Some(heap) = &delta.heap {
        map.insert(
            "heap".to_string(),
            Json::Array(heap.iter().map(cell_to_json).collect()),
        );
    }
    if let Some(iterating) = &delta.iterating {
        map.insert(
            "iterating".to_string(),
            Json::Array(
                iterating
                    .iter()
                    .map(|id| Json::Number(*id as f64))
                    .collect(),
            ),
        );
    }
    if let Some(func_refs) = &delta.func_refs {
        map.insert(
            "func_refs".to_string(),
            Json::Array(
                func_refs
                    .iter()
                    .map(|id| match id {
                        Some(id) => Json::Number(*id as f64),
                        None => Json::Null,
                    })
                    .collect(),
            ),
        );
    }
    Json::Object(map)
}

fn reentry_delta_to_json(reentry: &JoinReentry) -> Json {
    let mut map = BTreeMap::new();
    map.insert("site".to_string(), Json::Number(reentry.site as f64));
    map.insert("next".to_string(), Json::Number(reentry.next as f64));
    Json::Object(map)
}

fn json_to_reentry_delta(json: &Json) -> Result<JoinReentry, StateError> {
    let map = json.as_object()?;
    Ok(JoinReentry {
        site: Json::get(map, "site")?.as_u32()?,
        next: Json::get(map, "next")?.as_u32()?,
    })
}

pub fn json_to_continuation_delta(json: &Json) -> Result<ContinuationDelta, StateError> {
    let map = json.as_object()?;
    Ok(ContinuationDelta {
        frames: match map.get("frames") {
            None | Some(Json::Null) => Vec::new(),
            Some(value) => value
                .as_array()?
                .iter()
                .map(json_to_frame_delta)
                .collect::<Result<_, _>>()?,
        },
        stack: match map.get("stack") {
            None => None,
            Some(value) => Some(
                value
                    .as_array()?
                    .iter()
                    .map(json_to_value)
                    .collect::<Result<_, _>>()?,
            ),
        },
        pending: match map.get("pending") {
            None => None,
            Some(Json::Null) => Some(None),
            Some(value) => Some(Some(json_to_pending(value)?)),
        },
        status: match map.get("status") {
            None => None,
            Some(value) => Some(parse_status(value.as_str()?)?),
        },
        result: match map.get("result") {
            None => None,
            Some(Json::Null) => Some(None),
            Some(value) => Some(Some(json_to_value(value)?)),
        },
        try_stack: match map.get("try_stack") {
            None => None,
            Some(value) => Some(
                value
                    .as_array()?
                    .iter()
                    .map(json_to_try_handler)
                    .collect::<Result<_, _>>()?,
            ),
        },
        join: match map.get("join") {
            None => None,
            Some(Json::Null) => Some(None),
            Some(value) => Some(Some(json_to_join(value)?)),
        },
        reentries: match map.get("reentries") {
            None => None,
            Some(Json::Null) => Some(Vec::new()),
            Some(value) => Some(
                value
                    .as_array()?
                    .iter()
                    .map(json_to_reentry_delta)
                    .collect::<Result<_, _>>()?,
            ),
        },
        heap: match map.get("heap") {
            None => None,
            Some(Json::Null) => Some(Vec::new()),
            Some(value) => Some(
                value
                    .as_array()?
                    .iter()
                    .map(json_to_cell)
                    .collect::<Result<_, _>>()?,
            ),
        },
        iterating: match map.get("iterating") {
            None => None,
            Some(Json::Null) => Some(Vec::new()),
            Some(value) => Some(
                value
                    .as_array()?
                    .iter()
                    .map(|item| item.as_u32())
                    .collect::<Result<_, _>>()?,
            ),
        },
        func_refs: match map.get("func_refs") {
            None => None,
            Some(Json::Null) => Some(Vec::new()),
            Some(value) => Some(
                value
                    .as_array()?
                    .iter()
                    .map(|item| match item {
                        Json::Null => Ok(None),
                        number => Ok(Some(number.as_u32()?)),
                    })
                    .collect::<Result<_, _>>()?,
            ),
        },
    })
}

pub fn encode_continuation_delta(delta: &ContinuationDelta) -> Result<Vec<u8>, StateError> {
    Ok(continuation_delta_to_json(delta).stringify().into_bytes())
}

pub fn decode_continuation_delta(bytes: &[u8]) -> Result<ContinuationDelta, StateError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| StateError::InvalidEncoding("delta is not utf-8".to_string()))?;
    json_to_continuation_delta(&Json::parse(text)?)
}

fn frame_delta_to_json(delta: &FrameDelta) -> Json {
    let mut map = BTreeMap::new();
    map.insert("index".to_string(), Json::Number(delta.index as f64));
    if let Some(pc) = delta.pc {
        map.insert("pc".to_string(), Json::Number(pc as f64));
    }
    if !delta.locals.is_empty() {
        map.insert(
            "locals".to_string(),
            Json::Array(delta.locals.iter().map(local_patch_to_json).collect()),
        );
    }
    Json::Object(map)
}

fn json_to_frame_delta(json: &Json) -> Result<FrameDelta, StateError> {
    let map = json.as_object()?;
    Ok(FrameDelta {
        index: Json::get(map, "index")?.as_u32()?,
        pc: match map.get("pc") {
            None | Some(Json::Null) => None,
            Some(value) => Some(value.as_u32()?),
        },
        locals: match map.get("locals") {
            None | Some(Json::Null) => Vec::new(),
            Some(value) => value
                .as_array()?
                .iter()
                .map(json_to_local_patch)
                .collect::<Result<_, _>>()?,
        },
    })
}

fn local_patch_to_json(patch: &LocalPatch) -> Json {
    let mut map = BTreeMap::new();
    map.insert("slot".to_string(), Json::Number(patch.slot as f64));
    map.insert("value".to_string(), value_to_json(&patch.value));
    Json::Object(map)
}

fn json_to_local_patch(json: &Json) -> Result<LocalPatch, StateError> {
    let map = json.as_object()?;
    Ok(LocalPatch {
        slot: Json::get(map, "slot")?.as_u32()?,
        value: json_to_value(Json::get(map, "value")?)?,
    })
}

fn delta_cheaper_than_snapshot(delta: &ContinuationDelta, current: &Continuation) -> bool {
    delta_units(delta) < snapshot_units(current)
}

fn snapshot_units(continuation: &Continuation) -> usize {
    continuation
        .frames
        .iter()
        .map(|frame| frame.locals.len() + 2)
        .sum::<usize>()
        + continuation.stack.len()
        + continuation.heap.len()
        + 4
}

fn delta_units(delta: &ContinuationDelta) -> usize {
    delta
        .frames
        .iter()
        .map(|frame| frame.locals.len() + usize::from(frame.pc.is_some()))
        .sum::<usize>()
        + delta
            .stack
            .as_ref()
            .map(|stack| stack.len().max(1))
            .unwrap_or(0)
        + usize::from(delta.pending.is_some())
        + usize::from(delta.status.is_some())
        + usize::from(delta.result.is_some())
        + usize::from(delta.try_stack.is_some())
        + delta
            .join
            .as_ref()
            .map(|join| {
                join.as_ref()
                    .map(|state| state.branches.len().max(1))
                    .unwrap_or(1)
            })
            .unwrap_or(0)
        + usize::from(delta.reentries.is_some())
        + delta.heap.as_ref().map(|heap| heap.len()).unwrap_or(0)
        + usize::from(delta.iterating.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::continuation::{PendingOp, WaitKind};
    use crate::encode::{decode_continuation, encode_continuation};
    use std::collections::BTreeMap;

    use crate::value::{HeapCell, Value};

    fn start() -> Continuation {
        Continuation::start("gold-exec", "gold-hash", 1, "ts.subset.v1", 0, 2)
    }

    #[test]
    fn heap_mutation_is_a_delta_when_the_local_ref_is_unchanged() {
        let mut base = start();
        base.heap.push(HeapCell::Object(BTreeMap::from([(
            "x".into(),
            Value::Number(1.0),
        )])));
        base.frames[0].locals[0] = Value::Ref(0);
        base.frames[0].locals[1] = Value::Ref(0);
        let mut changed = base.clone();
        match &mut changed.heap[0] {
            HeapCell::Object(fields) => {
                fields.insert("x".into(), Value::Number(2.0));
            }
            _ => panic!("object"),
        }
        let delta = diff_continuation(&base, &changed).unwrap();
        assert!(delta.frames.is_empty());
        assert!(delta.heap.is_some());
        let applied = apply_continuation_delta(base, &delta).unwrap();
        assert_eq!(applied.heap, changed.heap);
        assert_eq!(applied.frames[0].locals[0], Value::Ref(0));
        assert_eq!(applied.frames[0].locals[1], Value::Ref(0));
    }

    #[test]
    fn durable_number_equality_drives_local_deltas() {
        let mut base = start();
        base.frames[0].locals[0] = Value::Number(f64::NAN);
        base.frames[0].locals[1] = Value::Number(0.0);
        let mut same_nan = base.clone();
        same_nan.frames[0].locals[0] = Value::Number(f64::from_bits(0x7ff8_0000_0000_0001));
        let delta = diff_continuation(&base, &same_nan).unwrap();
        assert!(delta.frames.is_empty());
        let mut signed = base.clone();
        signed.frames[0].locals[1] = Value::Number(-0.0);
        let delta = diff_continuation(&base, &signed).unwrap();
        assert_eq!(delta.frames.len(), 1);
        assert_eq!(delta.frames[0].locals.len(), 1);
        assert_eq!(delta.frames[0].locals[0].slot, 1);
    }

    #[test]
    fn first_persist_is_a_snapshot() {
        let current = start();
        let intent = persist_intent(None, &current, 0);
        assert_eq!(intent.kind, PersistKind::Snapshot);
        assert!(intent.materialize);
        assert!(intent.delta.is_none());
    }

    #[test]
    fn materialize_bound_forces_snapshot() {
        let base = start();
        let mut current = base.clone();
        current.frames[0].pc = 4;
        let intent = persist_intent(Some(&base), &current, MATERIALIZE_EVERY);
        assert_eq!(intent.kind, PersistKind::Snapshot);
        assert!(intent.materialize);
    }

    #[test]
    fn local_and_stack_changes_are_a_delta() {
        let base = start();
        let mut current = base.clone();
        current.frames[0].pc = 3;
        current.frames[0].locals[0] = Value::Number(42.0);
        current.stack = vec![Value::String("ok".into())];
        let intent = persist_intent(Some(&base), &current, 0);
        assert_eq!(intent.kind, PersistKind::Delta);
        assert!(!intent.materialize);
        let applied = apply_continuation_delta(base, intent.delta.as_ref().unwrap()).unwrap();
        assert_eq!(applied, current);
    }

    #[test]
    fn frame_topology_change_is_a_snapshot() {
        let base = start();
        let mut current = base.clone();
        current.frames.clear();
        current.status = ContinuationStatus::Completed;
        current.result = Some(Value::Number(1.0));
        let intent = persist_intent(Some(&base), &current, 0);
        assert_eq!(intent.kind, PersistKind::Snapshot);
    }

    #[test]
    fn patch_to_undefined_clears_slot_omission_resurrects() {
        let mut base = start();
        base.frames[0].locals[0] = Value::String("dead-payload".into());
        let mut omitted = base.clone();
        omitted.frames[0].pc = 8;
        let omit = diff_continuation(&base, &omitted).unwrap();
        assert!(omit
            .frames
            .iter()
            .all(|frame| frame.locals.iter().all(|patch| patch.slot != 0)));
        let resurrected = apply_continuation_delta(base.clone(), &omit).unwrap();
        assert_eq!(
            resurrected.frames[0].locals[0],
            Value::String("dead-payload".into())
        );

        let mut killed = base.clone();
        killed.frames[0].pc = 8;
        killed.frames[0].locals[0] = Value::Undefined;
        let kill = diff_continuation(&base, &killed).unwrap();
        assert!(kill.frames.iter().any(|frame| {
            frame
                .locals
                .iter()
                .any(|patch| patch.slot == 0 && patch.value == Value::Undefined)
        }));
        let applied = apply_continuation_delta(base, &kill).unwrap();
        assert_eq!(applied.frames[0].locals[0], Value::Undefined);
    }

    #[test]
    fn reconstruct_goldens_match_encode_continuation() {
        for name in [
            "store-local.json",
            "pending-wait.json",
            "clear-pending.json",
            "slot-undefined.json",
        ] {
            let path = format!(
                "{}/../../spec/fixtures/persist/{name}",
                env!("CARGO_MANIFEST_DIR")
            );
            let text =
                std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {path}: {err}"));
            let json = Json::parse(&text).unwrap();
            let map = json.as_object().unwrap();
            let base = decode_continuation(Json::get(map, "base").unwrap().stringify().as_bytes())
                .unwrap();
            let delta = json_to_continuation_delta(Json::get(map, "delta").unwrap()).unwrap();
            let expected =
                decode_continuation(Json::get(map, "expected").unwrap().stringify().as_bytes())
                    .unwrap();
            let applied = apply_continuation_delta(base, &delta).unwrap();
            assert_eq!(applied, expected, "{name}");
            assert_eq!(
                encode_continuation(&applied).unwrap(),
                encode_continuation(&expected).unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn pending_wait_diff_round_trips() {
        let base = start();
        let mut current = base.clone();
        current.frames[0].pc = 5;
        current.stack.clear();
        current.status = ContinuationStatus::Suspended;
        current.pending = Some(PendingOp::Wait {
            kind: WaitKind::Event {
                wait_id: "gold-exec:approved::4".into(),
                event_name: "approved".into(),
                correlation_key: None,
            },
        });
        let delta = diff_continuation(&base, &current).unwrap();
        let encoded = encode_continuation_delta(&delta).unwrap();
        let decoded = decode_continuation_delta(&encoded).unwrap();
        assert_eq!(apply_continuation_delta(base, &decoded).unwrap(), current);
    }
}
