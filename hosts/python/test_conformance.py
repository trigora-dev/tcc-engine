import tempfile
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from tcc_engine.compile import artifact_json, compile
from tcc_engine.store import Store
from conformance import assert_conformance, run_uninterrupted, kill_and_resume
from host import start_execution

FIRST = """
from trigora import program, effect, wait_for_event

@program
async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
"""

IF_ELSE = """
from trigora import program, effect

@program
async def run():
    flag = await effect("flag", lambda: 1)
    if flag:
        result = await effect("taken", lambda: 42)
        return result
    else:
        result = await effect("skipped", lambda: 99)
        return result
"""

LOOP = """
from trigora import program, effect

@program
async def run():
    go = await effect("go", lambda: 1)
    while go:
        x = await effect("x", lambda: 42)
        go = 0
        return x
    return 0
"""

SCOPES = """
from trigora import program, effect, wait_for_event

@program
async def run():
    outer = await effect("outer", lambda: 1)
    inner = await wait_for_event("go")
    return {"outer": outer, "inner": inner}
"""

MULTI = """
from trigora import program, effect, wait_for_event

@program
async def run():
    a = await effect("a", lambda: 1)
    b = await effect("b", lambda: 2)
    ev = await wait_for_event("go")
    return {"a": a, "b": b, "ev": ev}
"""

VALUES = """
from trigora import program, effect

@program
async def run():
    n = await effect("n", lambda: 3)
    if n == 3:
        yes = not False
        return {"ok": yes, "xs": [n, 1]}
    return {"ok": False, "xs": []}
"""

TRY = """
from trigora import program, effect

@program
async def run():
    try:
        raise Exception("boom")
    except Exception as e:
        ok = await effect("ok", lambda: 1)
        return ok
"""

FINALLY = """
from trigora import program, effect

@program
async def run():
    x = 0
    try:
        a = await effect("a", lambda: 1)
        x = a
    finally:
        x = 1
    return x
"""

RETRY = """
from trigora import program, effect

@program
async def run():
    x = await effect("flaky", lambda: 7)
    return x
"""

SLEEP = """
from trigora import program, sleep, effect

@program
async def run():
    await sleep(0)
    x = await effect("after", lambda: 1)
    return x
"""

CANCEL = """
from trigora import program, effect, wait_for_event

@program
async def run():
    a = await effect("a", lambda: 1)
    b = await wait_for_event("never")
    return b
"""

INVOKE = """
from trigora import program, invoke

@program
async def run():
    result = await invoke("child")
    return result
"""

CHILD = """
from trigora import program, effect

@program
async def run():
    result = await effect("child_work", lambda: 7)
    return result
"""


def test_first_example_recovery():
    assert_conformance(
        FIRST,
        effects={"generate": 42},
        expected_result={
            "t": "object",
            "v": {
                "approval": {"t": "string", "v": "ok"},
                "result": {"t": "number", "v": 42},
            },
        },
        expected_effect_log=["generate"],
        crash_ats=["after_persist_effect:generate", "after_wait_checkpoint"],
    )


def test_if_else_skips_untaken_branch():
    assert_conformance(
        IF_ELSE,
        effects={"flag": 1, "taken": 42, "skipped": 99},
        expected_result={"t": "number", "v": 42},
        expected_effect_log=["flag", "taken"],
        crash_ats=["after_persist_effect:flag", "after_persist_effect:taken", "after_persist_checkpoint:1"],
    )


def test_loops_journal_skip_same_key():
    assert_conformance(
        LOOP,
        effects={"go": 1, "x": 42},
        expected_result={"t": "number", "v": 42},
        expected_effect_log=["go", "x"],
        crash_ats=["after_persist_effect:go", "after_persist_effect:x"],
    )


def test_nested_scope_locals_survive_wait():
    assert_conformance(
        SCOPES,
        effects={"outer": 1},
        event_payload="ok",
        expected_result={
            "t": "object",
            "v": {
                "inner": {"t": "string", "v": "ok"},
                "outer": {"t": "number", "v": 1},
            },
        },
        expected_effect_log=["outer"],
        crash_ats=["after_persist_effect:outer", "after_wait_checkpoint"],
    )


def test_multiple_durable_operations():
    assert_conformance(
        MULTI,
        effects={"a": 1, "b": 2},
        event_payload="ok",
        expected_result={
            "t": "object",
            "v": {
                "a": {"t": "number", "v": 1},
                "b": {"t": "number", "v": 2},
                "ev": {"t": "string", "v": "ok"},
            },
        },
        expected_effect_log=["a", "b"],
        crash_ats=["after_persist_effect:a", "after_persist_effect:b", "after_wait_checkpoint"],
    )


def test_values_compare_and_build_structures():
    assert_conformance(
        VALUES,
        effects={"n": 3},
        expected_result={
            "t": "object",
            "v": {
                "ok": {"t": "bool", "v": True},
                "xs": {
                    "t": "array",
                    "v": [{"t": "number", "v": 3}, {"t": "number", "v": 1}],
                },
            },
        },
        expected_effect_log=["n"],
        crash_ats=["after_persist_effect:n"],
    )


def test_try_except_recovers_after_handler_effect():
    assert_conformance(
        TRY,
        effects={"ok": 1},
        expected_result={"t": "number", "v": 1},
        expected_effect_log=["ok"],
        crash_ats=["after_persist_effect:ok"],
    )


def test_try_finally_runs_cleanup():
    assert_conformance(
        FINALLY,
        effects={"a": 1},
        expected_result={"t": "number", "v": 1},
        expected_effect_log=["a"],
        crash_ats=["after_persist_effect:a"],
    )


def test_failed_effect_retries_same_key():
    clean = run_uninterrupted(RETRY, effects={"flaky": 7}, fail_counts={"flaky": 1})
    assert clean["status"] == "completed"
    assert clean["result"] == {"t": "number", "v": 7}
    assert clean["effectLog"] == ["flaky", "flaky"]
    recovered = kill_and_resume(
        RETRY, "after_persist_effect:flaky", effects={"flaky": 7}, fail_counts={"flaky": 1}
    )
    assert recovered["resumed"]["status"] == "completed"
    assert recovered["effectLog"] == ["flaky", "flaky"]


def test_sleep_survives_sigkill():
    assert_conformance(
        SLEEP,
        effects={"after": 1},
        expected_result={"t": "number", "v": 1},
        expected_effect_log=["after"],
        crash_ats=["after_register_timer", "after_wait_checkpoint", "after_persist_effect:after"],
    )


def test_cancel_at_durable_boundary():
    clean = run_uninterrupted(CANCEL, effects={"a": 1}, cancel=True, auto_deliver_event=False)
    assert clean["status"] == "cancelled"
    recovered = kill_and_resume(
        CANCEL, "after_wait_checkpoint", effects={"a": 1}, cancel=True, auto_deliver_event=False
    )
    assert recovered["resumed"]["status"] == "cancelled"
    assert recovered["effectLog"] == ["a"]


GATHER = """
from tcc_engine.primitives import program, effect, gather

@program
async def run():
    return await gather(effect("a", lambda: 1), effect("b", lambda: 2))
"""

RACE = """
from tcc_engine.primitives import program, effect, race

@program
async def run():
    return await race(effect("a", lambda: 1), effect("b", lambda: 2))
"""

RACE_FAIL = """
from tcc_engine.primitives import program, effect, race

@program
async def run():
    try:
        return await race(effect("bad", lambda: 1), effect("sibling", lambda: 2))
    except Exception:
        return "caught"
"""


def test_gather_returns_input_order_across_resume():
    assert_conformance(
        GATHER,
        effects={"a": 1, "b": 2},
        expected_result={
            "t": "array",
            "v": [{"t": "number", "v": 1}, {"t": "number", "v": 2}],
        },
        expected_effect_log=["a", "b"],
        crash_ats=["after_persist_effect:a", "after_persist_effect:b"],
    )


def test_race_lowest_index_wins_across_resume():
    assert_conformance(
        RACE,
        effects={"a": 1, "b": 2},
        expected_result={"t": "number", "v": 1},
        expected_effect_log=["a", "b"],
        crash_ats=["after_persist_effect:a", "after_persist_effect:b"],
    )


def test_rejecting_race_is_caught_once_across_resume():
    assert_conformance(
        RACE_FAIL,
        effects={"bad": "__fail__", "sibling": 2},
        expected_result={"t": "string", "v": "caught"},
        crash_ats=["after_persist_effect", "after_persist_checkpoint:2"],
    )


def test_child_invoke_recovers():
    child_json = artifact_json(compile(CHILD, filename="child.py"))
    assert_conformance(
        INVOKE,
        effects={"child_work": 7},
        child_artifacts={"child": child_json},
        expected_result={"t": "number", "v": 7},
        expected_effect_log=["child_work"],
        crash_ats=["after_create_child", "after_wait_checkpoint"],
    )


INPUT_PROGRAM = """
from tcc_engine.primitives import program
@program
async def run(input):
    return input
"""

NO_PARAM = """
from tcc_engine.primitives import program
@program
async def run():
    return 1
"""

ANALYZE = """
from tcc_engine.primitives import program
@program
async def run(input):
    return input["query"]
"""

PARENT_ANALYZE = """
from tcc_engine.primitives import program, invoke

@program
async def run():
    return await invoke("analyze", {"query": "hello"})
"""


def _start(source: str, **kwargs):
    db = Path(tempfile.mkdtemp()) / "tcc.db"
    return start_execution(db_path=str(db), artifact_json=artifact_json(compile(source)), **kwargs)


def test_python_start_requires_exact_arity():
    try:
        _start(INPUT_PROGRAM)
    except Exception as error:
        assert "missing" in str(error)
    else:
        raise AssertionError("run(input) started with no arguments")
    explicit = _start(INPUT_PROGRAM, program_args=[None])
    assert explicit["result"] == {"t": "null"}
    try:
        _start(NO_PARAM, program_args=[None])
    except Exception as error:
        assert "were given" in str(error)
    else:
        raise AssertionError("run() accepted an extra argument")
    child = artifact_json(
        compile(
            """
from tcc_engine.primitives import program
@program
async def run(a, b):
    return a
""",
            filename="two.py",
        )
    )
    try:
        _start(PARENT_ANALYZE, child_artifacts={"analyze": child})
    except Exception as error:
        assert "missing" in str(error)
    else:
        raise AssertionError("a short vector started a two-parameter child")


def test_invoke_input_is_stable_across_resume():
    child_json = artifact_json(compile(ANALYZE, filename="analyze.py"))
    assert_conformance(
        PARENT_ANALYZE,
        child_artifacts={"analyze": child_json},
        expected_result={"t": "string", "v": "hello"},
        crash_ats=["after_create_child", "after_wait_checkpoint"],
    )


def test_first_committed_child_args_stay_authoritative():
    store = Store(str(Path(tempfile.mkdtemp()) / "tcc.db"))
    store.put_artifact("h", "{}")
    base = {
        "parent_execution_id": "parent",
        "flow_name": "analyze",
        "artifact_hash": "h",
        "owner_token": "o",
        "lease_until": 1,
    }
    first = '[{"t":"string","v":"first"}]'
    store.enqueue_create_child({**base, "invoke_id": "same", "child_execution_id": "child", "args_json": first})
    store.flush_if_ungrouped()
    store.enqueue_create_child({**base, "invoke_id": "same", "child_execution_id": "child", "args_json": first})
    store.flush_if_ungrouped()
    assert store.get_child("same")["args_json"] == first
    try:
        store.enqueue_create_child(
            {**base, "invoke_id": "same", "child_execution_id": "child", "args_json": '[{"t":"string","v":"second"}]'}
        )
        store.flush_if_ungrouped()
    except Exception as error:
        assert "argument vector mismatch" in str(error)
    else:
        raise AssertionError("a conflicting argument vector was accepted")
    assert store.get_child("same")["args_json"] == first
    store.enqueue_create_child(
        {**base, "invoke_id": "absent", "child_execution_id": "child-absent", "args_json": "[]"}
    )
    store.flush_if_ungrouped()
    assert store.get_child("absent")["args_json"] is None
    store.enqueue_create_child(
        {
            **base,
            "invoke_id": "explicit-null",
            "child_execution_id": "child-null",
            "args_json": '[{"t":"null"}]',
        }
    )
    store.flush_if_ungrouped()
    assert store.get_child("explicit-null")["args_json"] == '[{"t":"null"}]'
    store.close()
