from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))

from kit import apply_delta, assert_conformance, resume_execution, start_execution, Store
from tcc_engine.compile import artifact_json, compile

CASES = json.loads((ROOT / "conformance" / "cases.json").read_text(encoding="utf-8"))
FIXTURES = ROOT / "spec" / "fixtures" / "persist"

FIRST = """
from trigora import effect, wait_for_event

async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
"""

OTHER = """
from trigora import effect, wait_for_event

async def run():
    result = await effect("generate", lambda: 99)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
"""


def test_reconstruct_goldens():
    listed = {Path(rel).name for rel in CASES["reconstruct"]}
    files = {path.name for path in FIXTURES.glob("*.json")}
    assert files == listed
    for rel in CASES["reconstruct"]:
        fixture = json.loads((ROOT / rel).read_text(encoding="utf-8"))
        applied = apply_delta(fixture["base"], fixture["delta"])
        applied["revision"] = fixture["expected"]["revision"]
        assert applied == fixture["expected"], rel


def test_first_example_crash_resume():
    first = next(item for item in CASES["semantic"] if item["id"] == "first-example")
    assert_conformance(
        FIRST,
        filename="first.py",
        expected_result={
            "t": "object",
            "v": {
                "approval": {"t": "string", "v": "ok"},
                "result": {"t": "number", "v": 42},
            },
        },
        crash_ats=first["crash_ats"],
    )


def test_artifact_pinning():
    stored = artifact_json(compile(FIRST, filename="first.py"))
    other = artifact_json(compile(OTHER, filename="other.py"))
    assert json.loads(stored)["envelope"]["artifact_hash"] != json.loads(other)["envelope"]["artifact_hash"]
    db_path = str(Path(tempfile.mkdtemp(prefix="tcc-kit-")) / "tcc.db")
    started = start_execution(db_path=db_path, artifact_json=stored, auto_deliver_event=False)
    assert started["status"] == "suspended"
    store = Store(db_path)
    execution = store.get_execution("first")
    saved = store.get_continuation("first")
    store.close()
    assert execution is not None and saved is not None
    assert execution["artifact_hash"] == json.loads(stored)["envelope"]["artifact_hash"]
    assert json.loads(saved["json"])["artifact_hash"] == execution["artifact_hash"]
    with pytest.raises(Exception, match="(?i)artifact"):
        resume_execution(db_path=db_path, artifact_json=other, event_payload="ok")
    resumed = resume_execution(db_path=db_path, event_payload="ok")
    assert resumed["status"] == "completed"


def test_generated_crash_resume():
    assert_conformance(
        """
from trigora import effect, wait_for_event

async def run():
    flag = await effect("generate", lambda: 1)
    if flag:
        approval = await wait_for_event("approved")
        return approval
    return 0
""",
        filename="branch.py",
        expected_result={"t": "string", "v": "ok"},
        crash_ats=["after_persist_checkpoint:1", "after_wait_checkpoint"],
    )
