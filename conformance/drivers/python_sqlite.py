"""Python SQLite adapter for host-conformance-v1."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
HOSTS_PYTHON = ROOT / "hosts" / "python"
if str(HOSTS_PYTHON) not in sys.path:
    sys.path.insert(0, str(HOSTS_PYTHON))

from host import resume_execution, start_execution
from tcc_engine.compile import artifact_json, compile
from tcc_engine.persist import apply_delta
from tcc_engine.store import Store

PY_WORKER = HOSTS_PYTHON / "worker.py"


def _encode_handle(execution_id: str, workspace: dict[str, str]) -> dict[str, str]:
    return {"executionId": execution_id, "workspace": json.dumps(workspace)}


def _decode_handle(handle: dict[str, Any]) -> dict[str, str]:
    return json.loads(handle["workspace"])


def _workspace() -> dict[str, str]:
    tmp = Path(tempfile.mkdtemp(prefix="tcc-py-kit-"))
    return {
        "dbPath": str(tmp / "tcc.db"),
        "logPath": str(tmp / "effects.log"),
        "configPath": str(tmp / "config.json"),
    }


def _read_effect_log(path: str) -> list[str]:
    file = Path(path)
    if not file.exists():
        return []
    text = file.read_text(encoding="utf-8").strip()
    return [] if not text else text.split("\n")


class PythonSqliteDriver:
    language = "python"

    def compile(self, source: str, filename: str) -> str:
        return artifact_json(compile(source, filename=filename))

    def apply_delta(
        self, base: dict[str, Any], delta: dict[str, Any]
    ) -> dict[str, Any]:
        return apply_delta(base, delta)

    def start(self, **input: Any) -> dict[str, Any]:
        workspace = _workspace()
        execution_id = input.get("executionId") or input.get("execution_id") or "first"
        result = start_execution(
            db_path=workspace["dbPath"],
            artifact_json=input["artifactJson"],
            execution_id=execution_id,
            effects=input.get("effects"),
            event_payload=input.get("eventPayload"),
            auto_deliver_event=input.get("autoDeliverEvent", True),
            completion_order=input.get("completionOrder") or "source",
            child_artifacts=input.get("childArtifacts"),
            cancel=input.get("cancel", False),
            effect_log_path=workspace["logPath"],
        )
        return {
            "status": result["status"],
            "result": result["result"],
            "handle": _encode_handle(execution_id, workspace),
        }

    def crash_at(self, **input: Any) -> dict[str, Any]:
        workspace = _workspace()
        execution_id = input.get("executionId") or input.get("execution_id") or "first"
        Path(workspace["configPath"]).write_text(
            json.dumps(
                {
                    "effects": input.get("effects") or {"generate": 42},
                    "event_payload": input.get("eventPayload"),
                    "auto_deliver_event": input.get("autoDeliverEvent", True),
                    "completion_order": input.get("completionOrder") or "source",
                    "child_artifacts": input.get("childArtifacts"),
                    "cancel": input.get("cancel", False),
                }
            ),
            encoding="utf-8",
        )
        env = {
            **os.environ,
            "TCC_MODE": "start",
            "TCC_DB_PATH": workspace["dbPath"],
            "TCC_ARTIFACT_JSON": input["artifactJson"],
            "TCC_CRASH_AT": input["crashAt"],
            "TCC_EFFECT_LOG_PATH": workspace["logPath"],
            "TCC_CONFIG_PATH": workspace["configPath"],
            "TCC_EXECUTION_ID": execution_id,
            "TCC_AUTO_EVENT": "0" if input.get("autoDeliverEvent") is False else "1",
        }
        killed = subprocess.run(
            [sys.executable, str(PY_WORKER)], env=env, capture_output=True, text=True
        )
        if killed.returncode not in (-signal.SIGKILL, 137):
            raise RuntimeError(
                f"expected SIGKILL, got {killed.returncode}: {killed.stderr}"
            )
        return _encode_handle(execution_id, workspace)

    def resume(self, **input: Any) -> dict[str, Any]:
        handle = input["handle"]
        workspace = _decode_handle(handle)
        result = resume_execution(
            db_path=workspace["dbPath"],
            artifact_json=input.get("artifactJson"),
            execution_id=handle["executionId"],
            effects=input.get("effects"),
            event_payload=input.get("eventPayload"),
            auto_deliver_event=input.get("autoDeliverEvent", True),
            completion_order=input.get("completionOrder") or "source",
            child_artifacts=input.get("childArtifacts"),
            cancel=input.get("cancel", False),
            effect_log_path=workspace["logPath"],
        )
        return {
            "status": result["status"],
            "result": result["result"],
            "handle": handle,
        }

    def read_continuation(self, handle: dict[str, Any]) -> dict[str, Any]:
        workspace = _decode_handle(handle)
        store = Store(workspace["dbPath"])
        try:
            saved = store.get_continuation(handle["executionId"])
            if saved is None:
                raise RuntimeError(f"missing continuation for {handle['executionId']}")
            return json.loads(saved["json"])
        finally:
            store.close()

    def effect_log(self, handle: dict[str, Any]) -> list[str]:
        return _read_effect_log(_decode_handle(handle)["logPath"])


def create_python_sqlite_driver() -> PythonSqliteDriver:
    return PythonSqliteDriver()


create_driver = create_python_sqlite_driver
