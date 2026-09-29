# Copyright (c) 2026 Trigora, Inc.
# SPDX-License-Identifier: BUSL-1.1
# See LICENSE for full terms.

"""Public Python mirror of the Node persistence benchmark suite.

Same scenario names and JSON field shapes as `bench/run.ts`.
Node remains the canonical public claim surface.

  python bench/python/run.py
  python bench/python/run.py persistence
  python bench/python/run.py recovery
  TCC_BENCH_SCALE=quick python bench/python/run.py

Optional targeted modes: persistence:healthy, persistence:group, persistence:replay,
recovery:live-state, recovery:wal. recovery:history and recovery:replay are Node-canonical.
"""

from __future__ import annotations

import json
import os
import platform
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "hosts" / "python"))

from tcc_engine.compile import artifact_json, compile
from tcc_engine.host import run_batch_on_store, run_on_store
from tcc_engine.store import Store

DISCLAIMER = (
    "These benchmarks characterize the portable TCC engine on the local host. "
    "They are not the research-prototype measurements and do not represent a hosted service."
)


def scale() -> dict:
    if os.environ.get("TCC_BENCH_SCALE") == "quick":
        return {
            "name": "quick",
            "warmup": 5,
            "measured": 20,
            "recover_samples": 20,
            "foreign": [0, 100, 1000],
            "concurrency": [1, 4, 8],
            "locals": [4, 256],
            "suffix": 10,
        }
    return {
        "name": "full",
        "warmup": 100,
        "measured": 1000,
        "recover_samples": 100,
        "foreign": [0, 100, 1000, 10000, 100000],
        "concurrency": [1, 2, 4, 8, 16, 32],
        "locals": [4, 256, 1024],
        "suffix": 10,
    }


def persist_modes() -> list[str]:
    raw = os.environ.get("TCC_BENCH_STORAGE")
    if raw in ("naive", "optimized"):
        return [raw]
    return ["naive", "optimized"]


def latency(samples: list[float]) -> dict:
    ordered = sorted(samples)

    def pct(p: float) -> float:
        if not ordered:
            return 0.0
        idx = min(len(ordered) - 1, max(0, int(__import__("math").ceil(p / 100 * len(ordered)) - 1)))
        return ordered[idx]

    return {
        "meanMs": round(statistics.fmean(ordered) if ordered else 0.0, 4),
        "medianMs": round(pct(50), 4),
        "p95Ms": round(pct(95), 4),
        "p99Ms": round(pct(99), 4),
        "samples": len(ordered),
    }


def db_path() -> str:
    return str(Path(tempfile.mkdtemp(prefix="tcc-bench-")) / "tcc.db")


def memory_bytes() -> int | None:
    try:
        return os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
    except (OSError, ValueError, AttributeError):
        return None


def dummy(execution_id: str, revision: int, locals_n: int) -> str:
    return json.dumps(
        {
            "artifact_hash": "bench-hash",
            "engine_format_version": 1,
            "execution_id": execution_id,
            "frames": [
                {
                    "func_id": 0,
                    "locals": [{"t": "number", "v": i} for i in range(locals_n)],
                    "pc": revision,
                }
            ],
            "language_semantics_version": "py.subset.v1",
            "pending": None,
            "result": None,
            "revision": revision,
            "stack": [],
            "status": "runnable",
            "try_stack": [],
        },
        separators=(",", ":"),
    )


def dummy_delta(pc: int) -> str:
    return json.dumps({"frames": [{"index": 0, "pc": pc}]}, separators=(",", ":"))


def accounting(metrics: dict) -> dict:
    samples = metrics["batchOccupancySamples"]
    avg = round(sum(samples) / len(samples), 2) if samples else 0
    avg_delta = (
        round(metrics["deltaBytesWritten"] / metrics["deltaCount"], 2) if metrics["deltaCount"] else 0
    )
    return {
        "bytesWritten": metrics["bytesWritten"],
        "snapshotCount": metrics["snapshotCount"],
        "deltaCount": metrics["deltaCount"],
        "averageDeltaSize": avg_delta,
        "commitCount": metrics["commitCount"],
        "batchOccupancyAvg": avg,
        "batchOccupancyMax": max(samples) if samples else 0,
        "deltaPayloadShare": round(metrics["deltaBytesWritten"] / metrics["bytesWritten"], 4) if metrics["bytesWritten"] else 0,
    }


def packing_fields(store: Store, persist: str) -> dict:
    if persist != "optimized":
        return {"packing": "n/a", "packingMinFullBytes": None, "packingMaxDeltaRatio": None}
    return {
        "packing": store.packing,
        "packingMinFullBytes": store.min_full_bytes,
        "packingMaxDeltaRatio": store.max_delta_ratio,
    }


FIRST = """
from trigora import program, effect, wait_for_event

@program
async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
"""


def healthy_path(cfg: dict) -> list[dict]:
    artifact = artifact_json(compile(FIRST, filename="first.py"))
    rows = []
    for persist in persist_modes():
        store = Store(db_path(), persist)
        store.reset_metrics()
        try:
            for i in range(cfg["warmup"]):
                run_on_store(
                    store,
                    artifact_json=artifact,
                    execution_id=f"w-{i}",
                    effects={"generate": 42},
                )
            store.reset_metrics()
            samples = []
            started = time.perf_counter()
            for i in range(cfg["measured"]):
                t0 = time.perf_counter()
                result = run_on_store(
                    store,
                    artifact_json=artifact,
                    execution_id=f"m-{i}",
                    effects={"generate": 42},
                )
                samples.append((time.perf_counter() - t0) * 1000)
                if result["status"] != "completed":
                    raise RuntimeError(result["status"])
            total_ms = (time.perf_counter() - started) * 1000
            rows.append(
                {
                    "scenario": "healthy_path_persistence",
                    "workload": "A",
                    "storage": persist,
                    **packing_fields(store, persist),
                    **latency(samples),
                    "executionsPerSec": round(cfg["measured"] / (total_ms / 1000), 2),
                    **accounting(store.metrics()),
                }
            )
        finally:
            store.close()
    return rows


def concurrency(cfg: dict) -> list[dict]:
    artifact = artifact_json(compile(FIRST, filename="first.py"))
    rows = []
    waves = max(1, cfg["measured"] // max(cfg["concurrency"]))
    for persist in persist_modes():
        for conc in cfg["concurrency"]:
            store = Store(db_path(), persist)
            store.reset_metrics()
            samples = []
            completed = 0
            try:
                for w in range(waves):
                    t0 = time.perf_counter()
                    inputs = [
                        {"artifact_json": artifact, "execution_id": f"c-{conc}-w{w}-n{n}", "effects": {"generate": 42}}
                        for n in range(conc)
                    ]
                    results = (
                        run_batch_on_store(store, inputs)
                        if persist == "optimized"
                        else [run_on_store(store, **item) for item in inputs]
                    )
                    for result in results:
                        if result["status"] != "completed":
                            raise RuntimeError(result["status"])
                        completed += 1
                    samples.append((time.perf_counter() - t0) * 1000)
                total_ms = sum(samples)
                rows.append(
                    {
                        "scenario": "concurrency_group_commit",
                        "storage": persist,
                        **packing_fields(store, persist),
                        "concurrency": conc,
                        "completed": completed,
                        **latency(samples),
                        "executionsPerSec": round(completed / (total_ms / 1000), 2) if total_ms else 0,
                        **accounting(store.metrics()),
                    }
                )
            finally:
                store.close()
    return rows


def wal_isolation(cfg: dict) -> list[dict]:
    rows = []
    suffix = cfg["suffix"]
    for foreign in cfg["foreign"]:
        store = Store(db_path(), "optimized")
        try:
            store.put_artifact("bench-hash", "{}")
            store.begin_group()
            for i in range(foreign):
                ident = f"foreign-{i}"
                store.create_execution(ident, "bench-hash", "owner-1", 1_000_000)
                store.commit_checkpoint(
                    ident, 1, dummy(ident, 1, 4), "runnable", "owner-1", kind="snapshot", materialize=True
                )
            store.create_execution("target", "bench-hash", "owner-1", 1_000_000)
            store.commit_checkpoint(
                "target", 1, dummy("target", 1, 4), "runnable", "owner-1", kind="snapshot", materialize=True
            )
            for revision in range(2, suffix + 2):
                store.commit_checkpoint(
                    "target",
                    revision,
                    dummy("target", revision, 4),
                    "runnable",
                    "owner-1",
                    kind="delta",
                    materialize=False,
                    delta_json=dummy_delta(revision),
                )
            store.end_group()
            plan = store.recover_plan("target")
            assert plan is not None
            assert plan["suffix_length"] == suffix
            assert plan["used_index"], plan["detail"]
            samples = []
            for _ in range(cfg["recover_samples"]):
                t0 = time.perf_counter()
                saved = store.get_continuation("target")
                samples.append((time.perf_counter() - t0) * 1000)
                assert saved is not None
            rows.append(
                {
                    "scenario": "wal_isolation",
                    "foreignCount": foreign,
                    "targetSuffix": suffix,
                    "suffixLength": plan["suffix_length"],
                    "usedIndex": plan["used_index"],
                    **latency(samples),
                    "retainedWalBytes": store.retained_wal_bytes(),
                    "liveStateBytes": store.retained_wal_bytes("target"),
                }
            )
        finally:
            store.close()
    return rows


def live_state(cfg: dict) -> list[dict]:
    rows = []
    for locals_n in cfg["locals"]:
        store = Store(db_path(), "optimized")
        try:
            store.put_artifact("bench-hash", "{}")
            store.create_execution("live", "bench-hash", "owner-1", 1_000_000)
            store.commit_checkpoint(
                "live", 1, dummy("live", 1, locals_n), "runnable", "owner-1", kind="snapshot", materialize=True
            )
            samples = []
            live_bytes = 0
            for _ in range(cfg["recover_samples"]):
                t0 = time.perf_counter()
                saved = store.get_continuation("live")
                samples.append((time.perf_counter() - t0) * 1000)
                assert saved is not None
                live_bytes = len(saved["json"])
            rows.append(
                {
                    "scenario": "live_state_scaling",
                    "locals": locals_n,
                    "liveStateBytes": live_bytes,
                    "retainedWalBytes": store.retained_wal_bytes("live"),
                    **latency(samples),
                }
            )
        finally:
            store.close()
    return rows


def healthy_path_replay_baseline(cfg: dict) -> list[dict]:
    artifact = artifact_json(compile(FIRST, filename="first.py"))
    rows: list[dict] = []
    models = ("replay", "optimized")
    for persist in models:
        store = Store(db_path(), persist)
        store.reset_metrics()
        try:
            for i in range(cfg["warmup"]):
                run_on_store(
                    store,
                    artifact_json=artifact,
                    execution_id=f"w-{i}",
                    effects={"generate": 42},
                )
            store.reset_metrics()
            samples = []
            started = time.perf_counter()
            for i in range(cfg["measured"]):
                t0 = time.perf_counter()
                result = run_on_store(
                    store,
                    artifact_json=artifact,
                    execution_id=f"m-{i}",
                    effects={"generate": 42},
                )
                samples.append((time.perf_counter() - t0) * 1000)
                if result["status"] != "completed":
                    raise RuntimeError(result["status"])
            total_ms = (time.perf_counter() - started) * 1000
            rows.append(
                {
                    "scenario": "healthy_path_replay_baseline",
                    "workload": "A",
                    "storage": persist,
                    "concurrency": 1,
                    "coordination": "single",
                    **latency(samples),
                    "executionsPerSec": round(cfg["measured"] / (total_ms / 1000), 2),
                    **accounting(store.metrics()),
                }
            )
        finally:
            store.close()
    waves = max(1, cfg["measured"] // max(cfg["concurrency"]))
    for persist in models:
        for conc in cfg["concurrency"]:
            store = Store(db_path(), persist)
            store.reset_metrics()
            samples = []
            completed = 0
            try:
                for w in range(waves):
                    t0 = time.perf_counter()
                    inputs = [
                        {"artifact_json": artifact, "execution_id": f"r-{conc}-w{w}-n{n}", "effects": {"generate": 42}}
                        for n in range(conc)
                    ]
                    results = run_batch_on_store(store, inputs)
                    for result in results:
                        if result["status"] != "completed":
                            raise RuntimeError(result["status"])
                        completed += 1
                    samples.append((time.perf_counter() - t0) * 1000)
                total_ms = sum(samples)
                rows.append(
                    {
                        "scenario": "healthy_path_replay_baseline",
                        "workload": "A",
                        "storage": persist,
                        "concurrency": conc,
                        "coordination": "wave",
                        "completed": completed,
                        **latency(samples),
                        "executionsPerSec": round(completed / (total_ms / 1000), 2) if total_ms else 0,
                        **accounting(store.metrics()),
                    }
                )
            finally:
                store.close()
    replay_median = {
        (row["workload"], row["concurrency"], row["coordination"]): row["medianMs"]
        for row in rows
        if row["storage"] == "replay"
    }
    for row in rows:
        if row["storage"] != "optimized":
            continue
        base = replay_median.get((row["workload"], row["concurrency"], row["coordination"]))
        row["overheadVsReplay"] = (
            round((row["medianMs"] - base) / base, 4) if base else None
        )
    return rows


ALIASES = {
    "concurrency": "persistence:group",
    "replay": "persistence:replay",
    "isolation": "recovery:wal",
}
KNOWN = {
    "all",
    "persistence",
    "recovery",
    "persistence:healthy",
    "persistence:group",
    "persistence:replay",
    "recovery:live-state",
    "recovery:wal",
}
USAGE = (
    "usage: python bench/python/run.py "
    "[all|persistence|recovery|persistence:healthy|persistence:group|persistence:replay|"
    "recovery:live-state|recovery:wal]"
)


def main() -> None:
    requested = sys.argv[1] if len(sys.argv) > 1 else "all"
    mode = ALIASES.get(requested, requested)
    if requested == "recovery:history" or mode == "recovery:history":
        print("recovery:history is Node-canonical; use: pnpm bench:recovery:history", file=sys.stderr)
        sys.exit(1)
    if requested == "recovery:replay" or mode == "recovery:replay":
        print("recovery:replay is Node-canonical; use: pnpm bench:recovery:replay", file=sys.stderr)
        sys.exit(1)
    if mode not in KNOWN:
        print(f"unknown mode: {requested}", file=sys.stderr)
        print(USAGE, file=sys.stderr)
        sys.exit(1)
    cfg = scale()
    started_at = time.time()
    git_sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=False).stdout.strip() or None
    version = next(
        line.split('"')[1]
        for line in (ROOT / "bindings" / "python" / "pyproject.toml").read_text().splitlines()
        if line.startswith("version = ")
    )
    print(DISCLAIMER)
    rows: list[dict] = []
    if mode in {"all", "persistence", "persistence:healthy"}:
        rows.extend(healthy_path(cfg))
    if mode in {"all", "persistence", "persistence:group"}:
        rows.extend(concurrency(cfg))
    if mode in {"all", "persistence", "persistence:replay"}:
        rows.extend(healthy_path_replay_baseline(cfg))
    if mode in {"all", "recovery", "recovery:live-state"}:
        rows.extend(live_state(cfg))
    if mode in {"all", "recovery", "recovery:wal"}:
        rows.extend(wal_isolation(cfg))
    finished_at = time.time()
    meta = {
        "disclaimer": DISCLAIMER, "host": "python", "scale": cfg["name"],
        "runtime": f"python {platform.python_version()}", "gitSha": git_sha,
        "engineVersion": version, "sqliteVersion": sqlite3.sqlite_version,
        "cpuModel": platform.processor() or platform.uname().processor or "unknown", "memoryBytes": memory_bytes(),
        "os": f"{platform.system()} {platform.release()}", "arch": platform.machine(),
        "startedAt": datetime.fromtimestamp(started_at, timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z"),
        "finishedAt": datetime.fromtimestamp(finished_at, timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z"),
        "durationMs": round((finished_at - started_at) * 1000),
    }
    print(json.dumps({"meta": meta, "results": rows}, indent=2))


if __name__ == "__main__":
    main()
