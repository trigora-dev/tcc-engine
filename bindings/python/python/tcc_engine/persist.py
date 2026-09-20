"""Semantic continuation deltas. Reconstruction is host packing, not instruction replay."""

from __future__ import annotations

import copy
import os
from typing import Any

MATERIALIZE_EVERY = 32

Continuation = dict[str, Any]
ContinuationDelta = dict[str, Any]

PYTHON_PACKING_DEFAULT = "follow"  # Full A-only/wave confirm did not beat follow.
PYTHON_ADAPTIVE_THRESHOLDS = {"min_full_bytes": 1024, "max_delta_ratio": 0.5}


def persist_mode_from_env() -> str:
    return "naive" if os.environ.get("TCC_PERSIST") == "naive" else "optimized"


def packing_from_env(fallback: str = PYTHON_PACKING_DEFAULT) -> str:
    value = os.environ.get("TCC_PERSIST_PACKING")
    if value in ("follow", "adaptive"):
        return value
    return fallback


def packing_thresholds_from_env(
    fallback: dict[str, float] | None = None,
) -> dict[str, float]:
    base = fallback or PYTHON_ADAPTIVE_THRESHOLDS
    min_raw = os.environ.get("TCC_PERSIST_MIN_FULL_BYTES")
    ratio_raw = os.environ.get("TCC_PERSIST_MAX_DELTA_RATIO")
    min_full_bytes = float(min_raw) if min_raw else float(base["min_full_bytes"])
    max_delta_ratio = float(ratio_raw) if ratio_raw else float(base["max_delta_ratio"])
    return {"min_full_bytes": min_full_bytes, "max_delta_ratio": max_delta_ratio}


def choose_packed_kind(
    *,
    must_materialize: bool,
    packing: str,
    full_bytes: int,
    delta_bytes: int | None,
    min_full_bytes: float,
    max_delta_ratio: float,
) -> str:
    if must_materialize:
        return "snapshot"
    if packing == "follow":
        return "delta"
    if delta_bytes is None:
        return "snapshot"
    if full_bytes < min_full_bytes:
        return "snapshot"
    if full_bytes == 0 or delta_bytes / full_bytes > max_delta_ratio:
        return "snapshot"
    return "delta"


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
