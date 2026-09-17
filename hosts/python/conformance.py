from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "frontends" / "python"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from compile import artifact_json, compile
from host import resume_execution, start_execution

WASM = ROOT / "target/wasm32-unknown-unknown/release/tcc_wasm.wasm"
NODE_WORKER = ROOT / "hosts/node/src/worker.ts"
PY_WORKER = Path(__file__).resolve().parent / "worker.py"


def compile_artifact(source: str, filename: str = "input.py") -> str:
    return artifact_json(compile(source, filename=filename))


def read_effect_log(path: Path) -> list[str]:
    if not path.exists():
        return []
    text = path.read_text(encoding="utf-8").strip()
    return [] if not text else text.split("\n")


def run_uninterrupted(
    source: str,
    *,
    filename: str = "input.py",
    effects: dict | None = None,
    event_payload=None,
    auto_deliver_event: bool = True,
    fail_counts: dict[str, int] | None = None,
    child_artifacts: dict[str, str] | None = None,
    cancel: bool = False,
    execution_id: str = "first",
) -> dict:
    tmp = Path(tempfile.mkdtemp(prefix="tcc-py-"))
    log_path = tmp / "effects.log"
    artifact_json_text = compile_artifact(source, filename)
    result = start_execution(
        db_path=str(tmp / "tcc.db"),
        artifact_json=artifact_json_text,
        effects=effects,
        event_payload=event_payload,
        auto_deliver_event=auto_deliver_event,
        fail_counts=fail_counts,
        child_artifacts=child_artifacts,
        cancel=cancel,
        execution_id=execution_id,
        effect_log_path=str(log_path),
    )
    result["effectLog"] = read_effect_log(log_path)
    result["artifactJson"] = artifact_json_text
    return result


def kill_and_resume(
    source: str,
    crash_at: str,
    *,
    filename: str = "input.py",
    effects: dict | None = None,
    event_payload=None,
    auto_deliver_event: bool = True,
    fail_counts: dict[str, int] | None = None,
    resume_fail_counts: dict[str, int] | None = None,
    child_artifacts: dict[str, str] | None = None,
    cancel: bool = False,
    resume_cancel: bool | None = None,
    execution_id: str = "first",
) -> dict:
    tmp = Path(tempfile.mkdtemp(prefix="tcc-py-kill-"))
    log_path = tmp / "effects.log"
    config_path = tmp / "config.json"
    db_path = tmp / "tcc.db"
    artifact_json_text = compile_artifact(source, filename)
    config_path.write_text(
        json.dumps(
            {
                "effects": effects or {"generate": 42},
                "failCounts": fail_counts,
                "fail_counts": fail_counts,
                "event_payload": event_payload,
                "auto_deliver_event": auto_deliver_event,
                "child_artifacts": child_artifacts,
                "cancel": cancel,
            }
        ),
        encoding="utf-8",
    )
    env = {
        **os.environ,
        "TCC_MODE": "start",
        "TCC_DB_PATH": str(db_path),
        "TCC_ARTIFACT_JSON": artifact_json_text,
        "TCC_CRASH_AT": crash_at,
        "TCC_EFFECT_LOG_PATH": str(log_path),
        "TCC_CONFIG_PATH": str(config_path),
        "TCC_EXECUTION_ID": execution_id,
        "TCC_AUTO_EVENT": "0" if auto_deliver_event is False else "1",
    }
    killed = subprocess.run([sys.executable, str(PY_WORKER)], env=env, capture_output=True, text=True)
    if killed.returncode not in (-signal.SIGKILL, 137):
        raise RuntimeError(f"expected SIGKILL, got {killed.returncode}: {killed.stderr}")
    config_path.write_text(
        json.dumps(
            {
                "effects": effects or {"generate": 42},
                "fail_counts": resume_fail_counts,
                "event_payload": event_payload,
                "auto_deliver_event": auto_deliver_event,
                "child_artifacts": child_artifacts,
                "cancel": resume_cancel if resume_cancel is not None else cancel,
            }
        ),
        encoding="utf-8",
    )
    resumed = resume_execution(
        db_path=str(db_path),
        effects=effects,
        event_payload=event_payload,
        auto_deliver_event=auto_deliver_event,
        fail_counts=resume_fail_counts,
        child_artifacts=child_artifacts,
        cancel=resume_cancel if resume_cancel is not None else cancel,
        execution_id=execution_id,
        effect_log_path=str(log_path),
    )
    return {"resumed": resumed, "effectLog": read_effect_log(log_path), "artifactJson": artifact_json_text}


def wasm_config(kwargs: dict, *, fail_counts_key: str = "fail_counts", cancel_key: str = "cancel") -> dict:
    payload = {
        "effects": kwargs.get("effects") or {"generate": 42},
        "failCounts": kwargs.get(fail_counts_key),
        "autoDeliverEvent": kwargs.get("auto_deliver_event", True),
        "childArtifacts": kwargs.get("child_artifacts"),
        "cancel": kwargs.get(cancel_key, False),
    }
    if kwargs.get("event_payload") is not None:
        payload["eventPayload"] = kwargs["event_payload"]
    return payload


def run_wasm_uninterrupted(artifact_json_text: str, **kwargs) -> dict:
    if not WASM.exists():
        return {"skipped": True}
    tmp = Path(tempfile.mkdtemp(prefix="tcc-wasm-"))
    log_path = tmp / "effects.log"
    config_path = tmp / "config.json"
    config_path.write_text(json.dumps(wasm_config(kwargs)), encoding="utf-8")
    env = {
        **os.environ,
        "TCC_MODE": "start",
        "TCC_DB_PATH": str(tmp / "tcc.db"),
        "TCC_WASM_PATH": str(WASM),
        "TCC_ARTIFACT_JSON": artifact_json_text,
        "TCC_EFFECT_LOG_PATH": str(log_path),
        "TCC_CONFIG_PATH": str(config_path),
        "TCC_EXECUTION_ID": kwargs.get("execution_id", "first"),
        "TCC_AUTO_EVENT": "0" if kwargs.get("auto_deliver_event") is False else "1",
    }
    completed = subprocess.run(
        [os.environ.get("NODE", "node"), "--experimental-sqlite", "--experimental-strip-types", str(NODE_WORKER)],
        env=env,
        capture_output=True,
        text=True,
        check=True,
    )
    result = json.loads(completed.stdout.strip().splitlines()[-1])
    result["effectLog"] = read_effect_log(log_path)
    return result


def kill_and_resume_wasm(artifact_json_text: str, crash_at: str, **kwargs) -> dict:
    if not WASM.exists():
        return {"skipped": True}
    tmp = Path(tempfile.mkdtemp(prefix="tcc-wasm-kill-"))
    log_path = tmp / "effects.log"
    config_path = tmp / "config.json"
    db_path = tmp / "tcc.db"
    config_path.write_text(json.dumps(wasm_config(kwargs)), encoding="utf-8")
    env = {
        **os.environ,
        "TCC_MODE": "start",
        "TCC_DB_PATH": str(db_path),
        "TCC_WASM_PATH": str(WASM),
        "TCC_ARTIFACT_JSON": artifact_json_text,
        "TCC_CRASH_AT": crash_at,
        "TCC_EFFECT_LOG_PATH": str(log_path),
        "TCC_CONFIG_PATH": str(config_path),
        "TCC_EXECUTION_ID": kwargs.get("execution_id", "first"),
        "TCC_AUTO_EVENT": "0" if kwargs.get("auto_deliver_event") is False else "1",
    }
    killed = subprocess.run(
        [os.environ.get("NODE", "node"), "--experimental-sqlite", "--experimental-strip-types", str(NODE_WORKER)],
        env=env,
        capture_output=True,
        text=True,
    )
    if killed.returncode not in (-signal.SIGKILL, 137):
        raise RuntimeError(f"wasm worker expected SIGKILL, got {killed.returncode}: {killed.stderr}")
    resume_kwargs = {**kwargs, "fail_counts": kwargs.get("resume_fail_counts"), "cancel": kwargs.get("resume_cancel", kwargs.get("cancel", False))}
    config_path.write_text(json.dumps(wasm_config(resume_kwargs)), encoding="utf-8")
    resume = subprocess.run(
        [os.environ.get("NODE", "node"), "--experimental-sqlite", "--experimental-strip-types", str(NODE_WORKER)],
        env={
            **env,
            "TCC_MODE": "resume",
            "TCC_CRASH_AT": "",
            "TCC_CONFIG_PATH": str(config_path),
        },
        capture_output=True,
        text=True,
        check=True,
    )
    result = json.loads(resume.stdout.strip().splitlines()[-1])
    return {"resumed": result, "effectLog": read_effect_log(log_path)}


def assert_conformance(
    source: str,
    *,
    expected_result,
    crash_ats: list[str],
    expected_effect_log: list[str] | None = None,
    cancel: bool = False,
    **kwargs,
) -> None:
    clean = run_uninterrupted(source, cancel=cancel, **kwargs)
    expected_status = "cancelled" if cancel else "completed"
    assert clean["status"] == expected_status, clean
    assert clean["result"] == expected_result, clean["result"]
    if expected_effect_log is not None:
        assert clean["effectLog"] == expected_effect_log, clean["effectLog"]
    for crash_at in crash_ats:
        recovered = kill_and_resume(source, crash_at, cancel=cancel, **kwargs)
        assert recovered["resumed"]["status"] == expected_status, recovered
        assert recovered["resumed"]["result"] == expected_result, recovered["resumed"]["result"]
        if expected_effect_log is not None:
            assert recovered["effectLog"] == expected_effect_log, recovered["effectLog"]
    wasm_clean = run_wasm_uninterrupted(clean["artifactJson"], cancel=cancel, **kwargs)
    if wasm_clean.get("skipped"):
        return
    assert wasm_clean["status"] == expected_status, wasm_clean
    assert wasm_clean["result"] == expected_result, wasm_clean["result"]
    if expected_effect_log is not None:
        assert wasm_clean["effectLog"] == expected_effect_log, wasm_clean["effectLog"]
    for crash_at in crash_ats:
        recovered = kill_and_resume_wasm(clean["artifactJson"], crash_at, cancel=cancel, **kwargs)
        if recovered.get("skipped"):
            return
        assert recovered["resumed"]["status"] == expected_status, recovered
        assert recovered["resumed"]["result"] == expected_result, recovered["resumed"]["result"]
        if expected_effect_log is not None:
            assert recovered["effectLog"] == expected_effect_log, recovered["effectLog"]
