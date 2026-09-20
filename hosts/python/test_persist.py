from pathlib import Path
import json
import sys
import tempfile
import os
import signal
import subprocess

sys.path.insert(0, str(Path(__file__).resolve().parent))

from tcc_engine.compile import artifact_json, compile
from tcc_engine.host import resume_execution, run_batch_on_store, run_on_store, start_execution
from tcc_engine.persist import MATERIALIZE_EVERY, apply_delta, choose_packed_kind, reconstruct_continuation
from tcc_engine.store import Store
from conformance import kill_and_resume, run_wasm_uninterrupted

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


def test_batch_confirms_after_commit_across_rounds():
    store = Store(_db(), "optimized")
    artifact = _artifact()
    inputs = [
        {"artifact_json": artifact, "execution_id": name, "effects": {"generate": 42}}
        for name in ("batch-a", "batch-b")
    ]
    rounds = []
    try:
        results = run_batch_on_store(store, inputs, lambda: rounds.append(1))
        assert len(rounds) == 4
        assert [result["status"] for result in results] == ["completed", "completed"]
        assert store.metrics()["commitCount"] == 4
        assert store.metrics()["batchOccupancySamples"] == [2] * 4
        for item in inputs:
            assert store.get_execution(item["execution_id"])["revision"] == 4
        store.begin_group()
        try:
            import pytest
            with pytest.raises(RuntimeError, match="open group"):
                run_on_store(store, **inputs[0])
        finally:
            store.abort_group()
    finally:
        store.close()


def test_failed_batch_confirms_no_pending_revision():
    import pytest
    db_path = _db()
    store = Store(db_path, "optimized")
    artifact = _artifact()
    rounds = []

    def fail_second():
        rounds.append(1)
        if len(rounds) == 2:
            store.db.execute("""CREATE TEMP TRIGGER fail_checkpoint BEFORE INSERT ON persist_wal
                WHEN NEW.revision = 2 BEGIN SELECT RAISE(ABORT, 'injected commit failure'); END""")

    try:
        with pytest.raises(Exception, match="injected commit failure"):
            run_batch_on_store(store, [
                {"artifact_json": artifact, "execution_id": name, "effects": {"generate": 42}}
                for name in ("fail-a", "fail-b")
            ], fail_second)
        for name in ("fail-a", "fail-b"):
            assert store.get_execution(name)["revision"] == 1
            assert store.get_continuation(name)["revision"] == 1
    finally:
        store.close()
    for name in ("fail-a", "fail-b"):
        result = resume_execution(db_path=db_path, execution_id=name, artifact_json=artifact, persist="optimized")
        assert result["status"] == "completed"


def test_sigkill_before_batch_commit_restores_prior_revision():
    db_path = _db()
    artifact = _artifact()
    script = """
import os, signal
from tcc_engine.host import run_batch_on_store
from tcc_engine.store import Store
store = Store(os.environ["TCC_DB"], "optimized")
rounds = [0]
def before_commit():
    rounds[0] += 1
    if rounds[0] == 2:
        os.kill(os.getpid(), signal.SIGKILL)
run_batch_on_store(store, [
    {"artifact_json": os.environ["TCC_ARTIFACT"], "execution_id": name, "effects": {"generate": 42}}
    for name in ("crash-a", "crash-b")
], before_commit)
"""
    child = subprocess.run([sys.executable, "-c", script], env={**os.environ, "TCC_DB": db_path, "TCC_ARTIFACT": artifact}, capture_output=True)
    assert child.returncode == -signal.SIGKILL, child.stderr.decode()
    store = Store(db_path, "optimized")
    try:
        for name in ("crash-a", "crash-b"):
            assert store.get_continuation(name)["revision"] == 1
    finally:
        store.close()
    for name in ("crash-a", "crash-b"):
        result = resume_execution(db_path=db_path, execution_id=name, artifact_json=artifact, persist="optimized")
        assert result["status"] == "completed"


def test_reconstruct_goldens():
    files = sorted(p for p in FIXTURES.iterdir() if p.suffix == ".json")
    assert len(files) >= 4
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


def test_naive_and_optimized_generated_programs():
    programs = [
        (
            """
from trigora import effect

async def run():
    flag = await effect("generate", lambda: 1)
    if flag:
        taken = await effect("taken", lambda: 42)
        return taken
    skipped = await effect("skipped", lambda: 99)
    return skipped
""",
            {"generate": 1, "taken": 42, "skipped": 99},
        ),
        (
            """
from trigora import effect, wait_for_event

async def run():
    dead = await effect("generate", lambda: "drop-me")
    live = dead
    approval = await wait_for_event("approved")
    return {"live": live, "approval": approval}
""",
            {"generate": "drop-me"},
        ),
        (
            """
from trigora import effect

async def run():
    try:
        raise Exception("boom")
    except Exception:
        ok = await effect("generate", lambda: 1)
        return ok
""",
            {"generate": 1},
        ),
    ]
    for source, effects in programs:
        artifact = artifact_json(compile(source))
        naive = start_execution(db_path=_db(), artifact_json=artifact, effects=effects, persist="naive")
        optimized = start_execution(
            db_path=_db(), artifact_json=artifact, effects=effects, persist="optimized"
        )
        assert naive["status"] == "completed", naive
        assert optimized["status"] == "completed", optimized
        assert optimized["result"] == naive["result"]


def test_optimized_wait_has_snapshot_and_delta():
    db_path = _db()
    first = start_execution(
        db_path=db_path, artifact_json=_artifact(), auto_deliver_event=False, persist="optimized", packing="follow"
    )
    assert first["status"] == "suspended"
    store = Store(db_path, "optimized", packing="follow")
    plan = store.recover_plan("first")
    records = store.wal_records("first")
    store.close()
    assert plan is not None
    assert plan["suffix_length"] <= MATERIALIZE_EVERY
    assert any(row["kind"] == "snapshot" for row in records)
    assert any(row["kind"] == "delta" for row in records)


def test_materialization_bound():
    db_path = _db()
    store = Store(db_path, "optimized", packing="follow")
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


INVOKE = """
from trigora import invoke

async def run():
    result = await invoke("child")
    return result
"""

CHILD = """
from trigora import effect

async def run():
    result = await effect("child_work", child_work)
    return result
"""


def _child_count(store: Store) -> int:
    row = store.db.execute("SELECT COUNT(*) AS n FROM children").fetchone()
    return int(row["n"])


def _invoke_inputs(names: list[str], parent: str, child: str):
    return [
        {
            "artifact_json": parent,
            "execution_id": name,
            "child_artifacts": {"child": child},
            "effects": {"child_work": 7},
        }
        for name in names
    ]


def test_batch_invoke_completes_with_independent_work():
    store = Store(_db(), "optimized")
    parent = artifact_json(compile(INVOKE, filename="parent.py"))
    child = artifact_json(compile(CHILD, filename="child.py"))
    independent = _artifact()
    try:
        results = run_batch_on_store(
            store,
            [
                *_invoke_inputs(["invoke-a", "invoke-b"], parent, child),
                {"artifact_json": independent, "execution_id": "solo", "effects": {"generate": 42}},
            ],
        )
        assert [result["status"] for result in results] == ["completed", "completed", "completed"]
        assert results[0]["result"] == {"t": "number", "v": 7}
        assert results[1]["result"] == {"t": "number", "v": 7}
        assert _child_count(store) == 2
        assert any(count >= 2 for count in store.metrics()["batchOccupancySamples"])
    finally:
        store.close()


def test_crash_before_invoke_commit_leaves_no_child():
    db_path = _db()
    parent = artifact_json(compile(INVOKE, filename="parent.py"))
    child = artifact_json(compile(CHILD, filename="child.py"))
    script = """
import os, signal
from tcc_engine.host import run_batch_on_store
from tcc_engine.store import Store
store = Store(os.environ["TCC_DB"], "optimized")
run_batch_on_store(store, [{
    "artifact_json": os.environ["TCC_PARENT"], "execution_id": "p1",
    "child_artifacts": {"child": os.environ["TCC_CHILD"]}, "effects": {"child_work": 7},
}], lambda: os.kill(os.getpid(), signal.SIGKILL))
"""
    proc = subprocess.run(
        [sys.executable, "-c", script],
        env={**os.environ, "TCC_DB": db_path, "TCC_PARENT": parent, "TCC_CHILD": child},
        capture_output=True,
    )
    assert proc.returncode == -signal.SIGKILL, proc.stderr.decode()
    store = Store(db_path, "optimized")
    try:
        assert store.get_execution("p1")["revision"] == 0
        assert store.get_continuation("p1") is None
        assert _child_count(store) == 0
    finally:
        store.close()


def test_commit_then_crash_before_ack_keeps_child():
    db_path = _db()
    parent = artifact_json(compile(INVOKE, filename="parent.py"))
    child = artifact_json(compile(CHILD, filename="child.py"))
    script = """
import os, signal
from tcc_engine.host import run_batch_on_store
from tcc_engine.store import Store
store = Store(os.environ["TCC_DB"], "optimized")
run_batch_on_store(store, [{
    "artifact_json": os.environ["TCC_PARENT"], "execution_id": "p1",
    "child_artifacts": {"child": os.environ["TCC_CHILD"]}, "effects": {"child_work": 7},
}], None, lambda: os.kill(os.getpid(), signal.SIGKILL))
"""
    proc = subprocess.run(
        [sys.executable, "-c", script],
        env={**os.environ, "TCC_DB": db_path, "TCC_PARENT": parent, "TCC_CHILD": child},
        capture_output=True,
    )
    assert proc.returncode == -signal.SIGKILL, proc.stderr.decode()
    store = Store(db_path, "optimized")
    try:
        assert store.get_continuation("p1")["revision"] == 1
        assert _child_count(store) == 1
    finally:
        store.close()
    resumed = resume_execution(
        db_path=db_path,
        artifact_json=parent,
        execution_id="p1",
        child_artifacts={"child": child},
        effects={"child_work": 7},
        persist="optimized",
    )
    assert resumed["status"] == "completed"
    assert resumed["result"] == {"t": "number", "v": 7}
    after = Store(db_path, "optimized")
    try:
        assert _child_count(after) == 1
    finally:
        after.close()


def test_duplicate_invoke_resume_one_child():
    db_path = _db()
    parent = artifact_json(compile(INVOKE, filename="parent.py"))
    child = artifact_json(compile(CHILD, filename="child.py"))
    first = start_execution(
        db_path=db_path,
        artifact_json=parent,
        execution_id="dup",
        child_artifacts={"child": child},
        effects={"child_work": 7},
        persist="optimized",
    )
    assert first["status"] == "completed"
    again = resume_execution(
        db_path=db_path,
        artifact_json=parent,
        execution_id="dup",
        child_artifacts={"child": child},
        effects={"child_work": 7},
        persist="optimized",
    )
    assert again["status"] == "completed"
    store = Store(db_path, "optimized")
    try:
        assert _child_count(store) == 1
    finally:
        store.close()


def test_replay_persist_and_resume():
    db_path = _db()
    artifact = _artifact()
    first = start_execution(
        db_path=db_path, artifact_json=artifact, persist="replay", auto_deliver_event=False
    )
    assert first["status"] == "suspended"
    store = Store(db_path, "replay")
    try:
        assert store.get_continuation("first") is None
        assert store.get_execution("first")["revision"] == 2
        assert all(row["kind"] == "history" for row in store.wal_records("first"))
    finally:
        store.close()
    resumed = resume_execution(db_path=db_path, artifact_json=artifact, persist="replay", event_payload="ok")
    assert resumed["status"] == "completed"


def test_replay_sigkill_does_not_reinvoke_effect():
    db_path = _db()
    artifact = _artifact()
    log_path = str(Path(db_path).parent / "effects.log")
    script = """
import os, signal
from tcc_engine.host import start_execution
def crash(hook, detail=None):
    if hook == "after_persist_checkpoint":
        os.kill(os.getpid(), signal.SIGKILL)
start_execution(
    db_path=os.environ["TCC_DB"],
    artifact_json=os.environ["TCC_ARTIFACT"],
    persist="replay",
    auto_deliver_event=False,
    effect_log_path=os.environ["TCC_LOG"],
    crash=crash,
)
"""
    proc = subprocess.run(
        [sys.executable, "-c", script],
        env={**os.environ, "TCC_DB": db_path, "TCC_ARTIFACT": artifact, "TCC_LOG": log_path},
        capture_output=True,
    )
    assert proc.returncode == -signal.SIGKILL, proc.stderr.decode()
    resumed = resume_execution(
        db_path=db_path,
        artifact_json=artifact,
        persist="replay",
        effect_log_path=log_path,
        event_payload="ok",
    )
    assert resumed["status"] == "completed"
    log = Path(log_path).read_text().strip().splitlines()
    assert log == ["generate"]


def test_host_on_event_reports_persist_restore_journal_and_child_without_blocking():
    events = []
    store = Store(_db(), "optimized", events.append)
    parent = artifact_json(compile(INVOKE, filename="invoke.py"))
    child = artifact_json(compile(CHILD, filename="child.py"))
    forbidden = {"json", "continuationJson", "artifactJson", "payload", "resultJson", "result_json"}

    def assert_clean(rows):
        for event in rows:
            assert forbidden.isdisjoint(event.keys()), event
            assert isinstance(event.get("engineVersion"), str) and event["engineVersion"]

    try:
        result = run_on_store(
            store,
            artifact_json=parent,
            execution_id="obs-parent",
            child_artifacts={"child": child},
            effects={"child_work": 7},
        )
        assert result["status"] == "completed"
        types = [event["type"] for event in events]
        assert "checkpoint.persisted" in types
        assert "checkpoint.materialized" in types
        assert "batch.committed" in types
        assert "child.created" in types
        assert "child.completed" in types
        store.get_continuation("obs-parent", observe_restore=True)
        assert any(event["type"] == "continuation.restored" for event in events)
        assert_clean(events)
    finally:
        store.close()

    journal_events = []
    journal_store = Store(_db(), "optimized", journal_events.append)
    first = _artifact()
    hash_ = json.loads(first)["envelope"]["artifact_hash"]
    try:
        journal_store.put_artifact(hash_, first)
        journal_store.create_execution("obs-journal", hash_, "owner-1", 1_000_000)
        journal_store.complete_effect("obs-journal", "generate", "k", json.dumps({"t": "number", "v": 42}))
        journaled = run_on_store(
            journal_store,
            artifact_json=first,
            execution_id="obs-journal",
            effects={"generate": 42},
        )
        assert journaled["status"] == "completed"
        assert any(event["type"] == "effect.journal_hit" for event in journal_events)
        assert_clean(journal_events)
    finally:
        journal_store.close()

    def boom(_event):
        raise RuntimeError("observer boom")

    throwing = Store(_db(), "optimized", boom)
    try:
        survived = run_on_store(
            throwing,
            artifact_json=_artifact(),
            execution_id="obs-throw",
            effects={"generate": 42},
        )
        assert survived["status"] == "completed"
    finally:
        throwing.close()

    errors = []
    fail_store = Store(_db(), "optimized", errors.append)
    try:
        fail_store.put_artifact("h", "{}")
        fail_store.create_execution("obs-fail", "h", "owner-1", 1_000_000)
        try:
            fail_store.commit_checkpoint("obs-fail", 2, _dummy("obs-fail", 2), "runnable", "owner-1")
            raise AssertionError("expected revision conflict")
        except RuntimeError as err:
            assert "revision conflict" in str(err)
        assert any(event["type"] == "runtime.error" for event in errors)
        assert_clean(errors)
    finally:
        fail_store.close()


def _compile(source: str, filename: str = "live.py") -> str:
    return artifact_json(compile(source, filename=filename))


def _load_head(db_path: str) -> dict:
    store = Store(db_path, "optimized")
    try:
        saved = store.get_continuation("first")
        assert saved is not None
        return {
            "json": saved["json"],
            "parsed": json.loads(saved["json"]),
            "records": store.wal_records("first"),
        }
    finally:
        store.close()


def _suspend(source: str, effects: dict, filename: str = "live.py") -> dict:
    db_path = _db()
    artifact = _compile(source, filename)
    result = start_execution(
        db_path=db_path,
        artifact_json=artifact,
        effects=effects,
        auto_deliver_event=False,
    )
    assert result["status"] == "suspended", result
    return {"db_path": db_path, "artifact": artifact, "result": result}


def test_reconstruct_golden_slot_undefined_does_not_resurrect():
    fixture = json.loads((FIXTURES / "slot-undefined.json").read_text())
    applied = apply_delta(fixture["base"], fixture["delta"])
    applied["revision"] = fixture["expected"]["revision"]
    assert applied == fixture["expected"]
    assert "DEAD_SHOULD_NOT_RESURRECT" not in json.dumps(applied)


def test_dead_scalar_undefined_after_later_wait():
    source = """
from trigora import effect, wait_for_event

async def run():
    dead = await effect("dead", lambda: "dead-scalar")
    live = await effect("live", lambda: "live-scalar")
    await wait_for_event("go")
    return live
"""
    head = _load_head(_suspend(source, {"dead": "dead-scalar", "live": "live-scalar"})["db_path"])
    locals_json = json.dumps(head["parsed"]["frames"][0]["locals"])
    assert "dead-scalar" not in locals_json
    assert "live-scalar" in locals_json


def test_large_dead_string_absent_from_committed_continuation():
    marker = "DEADBLOB_MARKER"
    blob = marker + ("x" * 1_000_000)
    source = """
from trigora import effect, wait_for_event

async def run():
    blob = await effect("blob", lambda: "unused")
    await wait_for_event("go")
    return 1
"""
    head = _load_head(_suspend(source, {"blob": blob})["db_path"])
    assert marker not in head["json"]
    assert len(head["json"]) < 50_000
    snapshot = next(row for row in head["records"] if row["kind"] == "snapshot")
    assert marker in snapshot["payload"]
    assert len(snapshot["payload"]) > 1_000_000
    assert len(head["json"]) < len(snapshot["payload"]) / 2


def test_live_value_survives_several_waits():
    source = """
from trigora import effect, wait_for_event

async def run():
    live = await effect("live", lambda: "stay-live")
    await wait_for_event("a")
    await wait_for_event("b")
    return live
"""
    started = _suspend(source, {"live": "stay-live"})
    assert "stay-live" in _load_head(started["db_path"])["json"]
    second = resume_execution(
        db_path=started["db_path"],
        artifact_json=started["artifact"],
        event_payload="ok",
        auto_deliver_event=False,
    )
    assert second["status"] == "suspended"
    assert "stay-live" in _load_head(started["db_path"])["json"]


def test_dead_after_taken_branch_is_gone():
    source = """
from trigora import effect, wait_for_event

async def run():
    flag = await effect("flag", lambda: 1)
    if flag:
        taken = await effect("taken", lambda: 42)
        dead = await effect("dead", lambda: "arm-dead")
        await wait_for_event("go")
        return taken
    else:
        skipped = await effect("skipped", lambda: 99)
        await wait_for_event("go")
        return skipped
"""
    locals_json = json.dumps(
        _load_head(_suspend(source, {"flag": 1, "taken": 42, "dead": "arm-dead", "skipped": 99})["db_path"])[
            "parsed"
        ]["frames"][0]["locals"],
        separators=(",", ":"),
    )
    assert "arm-dead" not in locals_json
    assert '"v":42' in locals_json
    assert '"v":99' not in locals_json


def test_loop_carried_value_remains():
    source = """
from trigora import effect, wait_for_event

async def run():
    acc = await effect("start", lambda: "loop-acc")
    go = await effect("go", lambda: 1)
    while go:
        tmp = await effect("tmp", lambda: "loop-tmp")
        await wait_for_event("tick")
        go = 0
    return acc
"""
    json_text = _load_head(_suspend(source, {"start": "loop-acc", "go": 1, "tmp": "loop-tmp"})["db_path"])["json"]
    assert "loop-acc" in json_text
    assert "loop-tmp" not in json_text


def test_try_except_keeps_catch_live_value_and_finally_drops_dead():
    keep = """
from trigora import effect, wait_for_event

async def run():
    try:
        keep = await effect("keep", lambda: "keep-me")
        await wait_for_event("in-try")
        raise Exception("boom")
    except Exception as e:
        return keep
"""
    assert "keep-me" in _load_head(_suspend(keep, {"keep": "keep-me"}, "keep.py")["db_path"])["json"]

    gone = """
from trigora import effect, wait_for_event

async def run():
    try:
        gone = await effect("gone", lambda: "drop-me")
    except Exception as e:
        return e
    finally:
        await wait_for_event("f")
    return 1
"""
    assert "drop-me" not in _load_head(_suspend(gone, {"gone": "drop-me"}, "gone.py")["db_path"])["json"]


def test_sigkill_after_compaction_resumes_live_result():
    source = """
from trigora import effect, wait_for_event

async def run():
    dead = await effect("dead", lambda: "dead-scalar")
    live = await effect("live", lambda: "live-scalar")
    await wait_for_event("go")
    return live
"""
    recovered = kill_and_resume(
        source,
        "after_wait_checkpoint",
        effects={"dead": "dead-scalar", "live": "live-scalar"},
        event_payload="ok",
    )
    assert recovered["resumed"]["status"] == "completed"
    assert recovered["resumed"]["result"] == {"t": "string", "v": "live-scalar"}


def test_native_and_wasm_agree_on_compacted_locals():
    source = """
from trigora import effect, wait_for_event

async def run():
    dead = await effect("dead", lambda: "dead-scalar")
    live = await effect("live", lambda: "live-scalar")
    await wait_for_event("go")
    return live
"""
    native = _suspend(source, {"dead": "dead-scalar", "live": "live-scalar"})
    native_json = json.dumps(_load_head(native["db_path"])["parsed"]["frames"][0]["locals"])
    wasm = run_wasm_uninterrupted(
        native["artifact"],
        effects={"dead": "dead-scalar", "live": "live-scalar"},
        auto_deliver_event=False,
    )
    if wasm.get("skipped"):
        return
    assert wasm["status"] == "suspended"
    wasm_json = json.dumps(json.loads(wasm["continuationJson"])["frames"][0]["locals"])
    assert "dead-scalar" not in native_json
    assert "live-scalar" in native_json
    assert "dead-scalar" not in wasm_json
    assert "live-scalar" in wasm_json


def test_choose_packed_kind_policy():
    assert (
        choose_packed_kind(
            must_materialize=True,
            packing="adaptive",
            full_bytes=10_000,
            delta_bytes=10,
            min_full_bytes=1024,
            max_delta_ratio=0.5,
        )
        == "snapshot"
    )
    assert (
        choose_packed_kind(
            must_materialize=False,
            packing="follow",
            full_bytes=100,
            delta_bytes=90,
            min_full_bytes=1024,
            max_delta_ratio=0.5,
        )
        == "delta"
    )
    assert (
        choose_packed_kind(
            must_materialize=False,
            packing="adaptive",
            full_bytes=500,
            delta_bytes=10,
            min_full_bytes=1024,
            max_delta_ratio=0.5,
        )
        == "snapshot"
    )
    assert (
        choose_packed_kind(
            must_materialize=False,
            packing="adaptive",
            full_bytes=4000,
            delta_bytes=200,
            min_full_bytes=1024,
            max_delta_ratio=0.5,
        )
        == "delta"
    )


def test_follow_packing_writes_delta():
    store = Store(_db(), "optimized", packing="follow")
    store.put_artifact("hash", "{}")
    store.create_execution("pack", "hash", "owner-1", 1_000_000)
    snapshot = _dummy("pack", 1)
    store.commit_checkpoint("pack", 1, snapshot, "runnable", "owner-1", kind="snapshot", materialize=True)
    nxt = _dummy("pack", 2)
    store.commit_checkpoint(
        "pack",
        2,
        nxt,
        "runnable",
        "owner-1",
        kind="delta",
        materialize=False,
        delta_json=json.dumps({"frames": [{"index": 0, "pc": 2}]}, separators=(",", ":")),
    )
    records = store.wal_records("pack")
    store.close()
    assert records[1]["kind"] == "delta"


def test_adaptive_packing_may_snapshot_delta_intent():
    store = Store(_db(), "optimized", packing="adaptive", min_full_bytes=10_000, max_delta_ratio=0.01)
    store.put_artifact("hash", "{}")
    store.create_execution("pack", "hash", "owner-1", 1_000_000)
    store.commit_checkpoint("pack", 1, _dummy("pack", 1), "runnable", "owner-1", kind="snapshot", materialize=True)
    nxt = _dummy("pack", 2)
    store.commit_checkpoint(
        "pack",
        2,
        nxt,
        "runnable",
        "owner-1",
        kind="delta",
        materialize=False,
        delta_json=json.dumps({"frames": [{"index": 0, "pc": 2}]}, separators=(",", ":")),
    )
    records = store.wal_records("pack")
    saved = store.get_continuation("pack")
    store.close()
    assert records[1]["kind"] == "snapshot"
    assert records[1]["payload"] == nxt
    assert json.loads(saved["json"])["revision"] == 2


def test_adaptive_extra_snapshots_keep_suffix_bound():
    store = Store(_db(), "optimized", packing="adaptive", min_full_bytes=10_000, max_delta_ratio=0.01)
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
    plan = store.recover_plan("bound")
    store.close()
    assert plan is not None
    assert plan["suffix_length"] <= MATERIALIZE_EVERY

