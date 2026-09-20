"""Semantic continuation deltas. Reconstruction is host packing, not instruction replay."""

from __future__ import annotations

import copy
import os
from typing import Any

MATERIALIZE_EVERY = 32

Continuation = dict[str, Any]
ContinuationDelta = dict[str, Any]


def persist_mode_from_env() -> str:
    return "naive" if os.environ.get("TCC_PERSIST") == "naive" else "optimized"


def apply_delta(base: Continuation, delta: ContinuationDelta) -> Continuation:
    next_ = copy.deepcopy(base)
    frames = next_.get("frames") or []
    for frame_delta in delta.get("frames") or []:
        index = int(frame_delta["index"])
        if index < 0 or index >= len(frames):
            raise RuntimeError(f"delta frame index {index} is out of range")
        frame = frames[index]
        if "pc" in frame_delta and frame_delta["pc"] is not None:
            frame["pc"] = frame_delta["pc"]
        locals_ = frame.get("locals") or []
        for patch in frame_delta.get("locals") or []:
            slot = int(patch["slot"])
            if slot < 0 or slot >= len(locals_):
                raise RuntimeError(f"delta local slot {slot} is out of range")
            locals_[slot] = patch["value"]
        frame["locals"] = locals_
    next_["frames"] = frames
    if "stack" in delta:
        next_["stack"] = delta["stack"] or []
    if "pending" in delta:
        next_["pending"] = delta["pending"]
    if "status" in delta:
        next_["status"] = delta["status"]
    if "result" in delta:
        next_["result"] = delta["result"]
    if "try_stack" in delta:
        next_["try_stack"] = delta["try_stack"] or []
    return next_


def reconstruct_continuation(
    snapshot: Continuation, deltas: list[ContinuationDelta], revision: int
) -> Continuation:
    current = snapshot
    for delta in deltas:
        current = apply_delta(current, delta)
    current["revision"] = revision
    return current
