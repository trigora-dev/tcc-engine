"""Internal engineering persist benches. Not the public claim surface.

Use `python bench/python/run.py` for the documented public suite.

  python hosts/python/bench_persist.py
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
from tcc_engine.persist import apply_delta
from tcc_engine.store import Store

FIRST = """
from trigora import program, effect, wait_for_event

@program
async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
"""


def _db() -> str:
    return str(Path(tempfile.mkdtemp(prefix="tcc-internal-")) / "tcc.db")


def micro_apply_delta(iters: int = 5000) -> dict:
    base = {
        "artifact_hash": "h",
        "engine_format_version": 1,
        "execution_id": "m",
        "frames": [{"func_id": 0, "locals": [{"t": "undefined"}] * 32, "pc": 0}],
        "language_semantics_version": "py.subset.v1",
        "pending": None,
        "result": None,
        "revision": 0,
        "stack": [],
        "status": "runnable",
        "try_stack": [],
    }
    delta = {"frames": [{"index": 0, "pc": 3, "locals": [{"slot": 1, "value": {"t": "number", "v": 1}}]}]}
    started = time.perf_counter()
    for _ in range(iters):
        apply_delta(base, delta)
    return {"kind": "micro_apply_delta", "iters": iters, "ms": round((time.perf_counter() - started) * 1000, 2)}


def smoke(persist: str, iters: int = 10) -> dict:
    artifact = artifact_json(compile(FIRST, filename="first.py"))
    started = time.perf_counter()
    for i in range(iters):
        result = start_execution(
            db_path=_db(),
            artifact_json=artifact,
            persist=persist,
            execution_id=f"e-{i}",
            effects={"generate": 42},
        )
        if result["status"] != "completed":
            raise RuntimeError(result["status"])
    return {
        "kind": "internal_healthy_smoke",
        "storage": persist,
        "iters": iters,
        "ms": round((time.perf_counter() - started) * 1000, 2),
    }


def main() -> None:
    print("internal persist bench — not for publication; see bench/README.md")
    rows = [micro_apply_delta(), smoke("naive"), smoke("optimized")]
    # Touch Store metrics path once.
    store = Store(_db(), "optimized")
    store.reset_metrics()
    rows.append({"kind": "metrics_zero", **store.metrics()})
    store.close()
    print(json.dumps(rows, indent=2))


if __name__ == "__main__":
    main()
