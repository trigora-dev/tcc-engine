from pathlib import Path
import json
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))

from tcc_engine.compile import artifact_json, compile
from tcc_engine.host import resume_execution, start_execution
from tcc_engine.persist import MATERIALIZE_EVERY, apply_delta, reconstruct_continuation
from tcc_engine.store import Store

FIRST = """
from trigora import effect, wait_for_event

async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
"""

FIXTURES = Path(__file__).resolve().parents[1].parent / "spec" / "fixtures" / "persist"


def _db() -> str:
    return str(Path(tempfile.mkdtemp(prefix="tcc-persist-")) / "tcc.db")


def _artifact() -> str:
    return artifact_json(compile(FIRST, filename="first.py"))


def _dummy(execution_id: str, revision: int) -> str:
    return json.dumps(
        {
            "artifact_hash": "hash",
            "engine_format_version": 1,
            "execution_id": execution_id,
            "frames": [{"func_id": 0, "locals": [{"t": "undefined"}], "pc": revision}],
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


def test_reconstruct_goldens():
    files = sorted(p for p in FIXTURES.iterdir() if p.suffix == ".json")
    assert len(files) >= 3
    for path in files:
        fixture = json.loads(path.read_text())
        applied = apply_delta(fixture["base"], fixture["delta"])
        applied["revision"] = fixture["expected"]["revision"]
        assert applied == fixture["expected"], path.name
        assert (
            reconstruct_continuation(fixture["base"], [fixture["delta"]], fixture["expected"]["revision"])
            == fixture["expected"]
        )


def test_naive_and_optimized_same_result():
    artifact = _artifact()
    naive = start_execution(db_path=_db(), artifact_json=artifact, persist="naive")
    optimized = start_execution(db_path=_db(), artifact_json=artifact, persist="optimized")
    assert naive["status"] == "completed"
    assert optimized["status"] == "completed"
    assert optimized["result"] == naive["result"]
    assert optimized["revision"] == naive["revision"]


def test_optimized_wait_has_snapshot_and_delta():
    db_path = _db()
    first = start_execution(
        db_path=db_path, artifact_json=_artifact(), auto_deliver_event=False, persist="optimized"
    )
    assert first["status"] == "suspended"
    store = Store(db_path, "optimized")
    plan = store.recover_plan("first")
    records = store.wal_records("first")
    store.close()
    assert plan is not None
    assert plan["suffix_length"] <= MATERIALIZE_EVERY
    assert any(row["kind"] == "snapshot" for row in records)
    assert any(row["kind"] == "delta" for row in records)


def test_materialization_bound():
    db_path = _db()
    store = Store(db_path, "optimized")
    store.put_artifact("hash", "{}")
    store.create_execution("bound", "hash", "owner-1", 1_000_000)
    for revision in range(1, MATERIALIZE_EVERY + 3):
        store.commit_checkpoint(
            "bound",
            revision,
            _dummy("bound", revision),
            "runnable",
            "owner-1",
            kind="snapshot" if revision == 1 else "delta",
            materialize=revision == 1,
            delta_json=json.dumps({"frames": [{"index": 0, "pc": revision}]}, separators=(",", ":")),
        )
    head = store.exec_head("bound")
    plan = store.recover_plan("bound")
    records = store.wal_records("bound")
    saved = store.get_continuation("bound")
    store.close()
    assert head is not None
    assert plan is not None
    assert plan["suffix_length"] <= MATERIALIZE_EVERY
    assert head["revision"] - head["snapshot_revision"] == plan["suffix_length"]
    assert sum(1 for row in records if row["kind"] == "snapshot") >= 2
    assert json.loads(saved["json"])["revision"] == MATERIALIZE_EVERY + 2


def test_recovery_does_not_scan_foreign_wal():
    db_path = _db()
    store = Store(db_path, "optimized")
    store.put_artifact("hash", "{}")
    store.begin_group()
    for i in range(200):
        ident = f"foreign-{i}"
        store.create_execution(ident, "hash", "owner-1", 1_000_000)
        store.commit_checkpoint(
            ident,
            1,
            _dummy(ident, 1),
            "runnable",
            "owner-1",
            kind="snapshot",
            materialize=True,
        )
    store.create_execution("target", "hash", "owner-1", 1_000_000)
    store.commit_checkpoint(
        "target", 1, _dummy("target", 1), "runnable", "owner-1", kind="snapshot", materialize=True
    )
    store.commit_checkpoint(
        "target",
        2,
        _dummy("target", 2),
        "runnable",
        "owner-1",
        kind="delta",
        materialize=False,
        delta_json=json.dumps({"frames": [{"index": 0, "pc": 2}]}, separators=(",", ":")),
    )
    store.end_group()
    plan = store.recover_plan("target")
    saved = store.get_continuation("target")
    own = len(store.wal_records("target"))
    total = store.wal_row_count()
    store.close()
    assert plan is not None
    assert plan["suffix_length"] == 1
    assert own == 2
    assert total >= 202
    assert plan["used_index"], plan["detail"]
    assert json.loads(saved["json"])["frames"][0]["pc"] == 2


def test_optimized_resume_matches_naive():
    artifact = _artifact()

    def run(persist: str):
        db_path = _db()
        first = start_execution(
            db_path=db_path,
            artifact_json=artifact,
            auto_deliver_event=False,
            persist=persist,
        )
        assert first["status"] == "suspended"
        return resume_execution(db_path=db_path, event_payload="ok", persist=persist)

    naive = run("naive")
    optimized = run("optimized")
    assert naive["status"] == "completed"
    assert optimized["status"] == "completed"
    assert optimized["result"] == naive["result"]
