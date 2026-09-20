"""Internal naive vs optimized persist benches. Not product numbers.

Run from the repo after a native install:

  .venv/bin/python hosts/python/bench_persist.py
"""

from __future__ import annotations

import json
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from tcc_engine.compile import artifact_json, compile
from tcc_engine.host import start_execution
from tcc_engine.store import Store

FIRST = """
from trigora import effect, wait_for_event

async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
"""

LARGE = """
from trigora import effect, wait_for_event

async def run():
    a = await effect("a", lambda: 1)
    b = await effect("b", lambda: 2)
    c = await effect("c", lambda: 3)
    d = await effect("d", lambda: 4)
    approval = await wait_for_event("approved")
    return {"a": a, "b": b, "c": c, "d": d, "approval": approval}
"""


def _db() -> str:
    return str(Path(tempfile.mkdtemp(prefix="tcc-bench-")) / "tcc.db")


def _dummy(execution_id: str, revision: int, locals_n: int) -> str:
    return json.dumps(
        {
            "artifact_hash": "hash",
            "engine_format_version": 1,
            "execution_id": execution_id,
            "frames": [
                {
                    "func_id": 0,
                    "locals": [{"t": "number", "v": 1}] * locals_n,
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


def healthy_path(label: str, source: str, persist: str, iters: int) -> dict:
    artifact = artifact_json(compile(source, filename=f"{label}.py"))
    started = time.perf_counter()
    for i in range(iters):
        result = start_execution(
            db_path=_db(),
            artifact_json=artifact,
            persist=persist,
            execution_id=f"exec-{i}",
            effects={"generate": 42} if label == "first-example" else {"a": 1, "b": 2, "c": 3, "d": 4},
        )
        if result["status"] != "completed":
            raise RuntimeError(f"{label} {persist} status {result['status']}")
    ms = (time.perf_counter() - started) * 1000
    return {
        "host": "python",
        "storage": persist,
        "workload": label,
        "iters": iters,
        "ms": round(ms, 2),
        "perSec": round(iters / (ms / 1000), 2),
    }


def recovery_vs_foreign_wal(foreign_count: int) -> dict:
    db_path = _db()
    store = Store(db_path, "optimized")
    store.put_artifact("hash", "{}")
    store.begin_group()
    for i in range(foreign_count):
        ident = f"foreign-{i}"
        store.create_execution(ident, "hash", "owner-1", 1_000_000)
        store.commit_checkpoint(
            ident, 1, _dummy(ident, 1, 4), "runnable", "owner-1", kind="snapshot", materialize=True
        )
    store.create_execution("target", "hash", "owner-1", 1_000_000)
    store.commit_checkpoint(
        "target", 1, _dummy("target", 1, 4), "runnable", "owner-1", kind="snapshot", materialize=True
    )
    store.end_group()
    started = time.perf_counter()
    saved = store.get_continuation("target")
    ms = (time.perf_counter() - started) * 1000
    plan = store.recover_plan("target")
    retained = store.retained_wal_bytes()
    own = store.retained_wal_bytes("target")
    store.close()
    if saved is None or plan is None:
        raise RuntimeError("missing target continuation")
    return {
        "host": "python",
        "storage": "optimized",
        "foreignCount": foreign_count,
        "recoverMs": round(ms, 4),
        "suffixLength": plan["suffix_length"],
        "usedIndex": plan["used_index"],
        "retainedWalBytes": retained,
        "liveStateBytes": own,
    }


def recovery_vs_live_state(locals_n: int) -> dict:
    db_path = _db()
    store = Store(db_path, "optimized")
    store.put_artifact("hash", "{}")
    store.create_execution("live", "hash", "owner-1", 1_000_000)
    store.commit_checkpoint(
        "live", 1, _dummy("live", 1, locals_n), "runnable", "owner-1", kind="snapshot", materialize=True
    )
    started = time.perf_counter()
    saved = store.get_continuation("live")
    ms = (time.perf_counter() - started) * 1000
    live_bytes = 0 if saved is None else len(saved["json"])
    retained = store.retained_wal_bytes("live")
    store.close()
    return {
        "host": "python",
        "storage": "optimized",
        "locals": locals_n,
        "recoverMs": round(ms, 4),
        "liveStateBytes": live_bytes,
        "retainedWalBytes": retained,
    }


def main() -> None:
    rows = [
        healthy_path("first-example", FIRST, "naive", 10),
        healthy_path("first-example", FIRST, "optimized", 10),
        healthy_path("larger-locals", LARGE, "naive", 10),
        healthy_path("larger-locals", LARGE, "optimized", 10),
        recovery_vs_foreign_wal(50),
        recovery_vs_foreign_wal(400),
        recovery_vs_live_state(4),
        recovery_vs_live_state(256),
    ]
    print("tcc persist bench (internal; not a paper claim)")
    print(json.dumps(rows, indent=2))


if __name__ == "__main__":
    main()
