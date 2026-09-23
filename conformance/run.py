"""Host-conformance-v1 executor over the shared cases.json corpus."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent


def load_cases() -> dict[str, Any]:
    return json.loads((HERE / "cases.json").read_text(encoding="utf-8"))


def _load_program(spec: dict[str, Any]) -> tuple[str, str]:
    if spec.get("file"):
        path = HERE / spec["file"]
        return path.read_text(encoding="utf-8"), spec.get("filename") or path.name
    if "source" not in spec:
        raise RuntimeError("program spec needs file or source")
    return spec["source"], spec.get("filename") or "input.py"


def _program_for(item: dict[str, Any], language: str) -> dict[str, Any]:
    if language == "python":
        spec = item.get("python")
    elif language == "rust":
        spec = item.get("rust")
    else:
        spec = item.get("typescript")
    if spec is None:
        raise RuntimeError(f"case {item['id']} has no {language} program")
    return spec


def _languages(item: dict[str, Any], driver: Any) -> list[str]:
    languages = []
    if item.get("compiler") == "typescript":
        languages.append("typescript")
    elif driver.language == "typescript" and item.get("typescript"):
        languages.append("typescript")
    elif driver.language == "python" and item.get("python"):
        languages.append("python")
    if item.get("rust"):
        languages.append("rust")
    return languages


def _run_reconstruct(driver: Any, cases: dict[str, Any]) -> None:
    fixtures = ROOT / "spec" / "fixtures" / "persist"
    listed = {Path(rel).name for rel in cases["reconstruct"]}
    files = {path.name for path in fixtures.glob("*.json")}
    assert files == listed, "reconstruct fixture listing"
    for rel in cases["reconstruct"]:
        fixture = json.loads((ROOT / rel).read_text(encoding="utf-8"))
        applied = driver.apply_delta(fixture["base"], fixture["delta"])
        applied["revision"] = fixture["expected"]["revision"]
        assert applied == fixture["expected"], rel


def _compile_typescript_file(path: Path) -> str:
    completed = __import__("subprocess").run(
        [
            "node",
            "--experimental-strip-types",
            str(HERE / "compile-ts.ts"),
            str(path),
        ],
        check=True,
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    return completed.stdout


def _child_artifacts(item: dict[str, Any]) -> dict[str, str] | None:
    children = item.get("children") or {}
    if not children:
        return None
    artifacts = {}
    for name, spec in children.items():
        path = HERE / spec["file"]
        artifacts[name] = _compile_typescript_file(path)
    return artifacts


def _compile_rust_file(path: Path) -> str:
    import os
    import subprocess

    root = HERE.parent
    binary = os.environ.get("TCC_RUSTC") or str(root / "target" / "release" / "tcc-rust-compile")
    completed = subprocess.run(
        [binary, str(path)],
        check=False,
        capture_output=True,
        text=True,
        cwd=root,
    )
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr or completed.stdout or f"rust compiler failed ({binary})")
    return completed.stdout.strip()


def _compile_spec(driver: Any, spec: dict[str, Any], language: str) -> str:
    if language == "rust":
        if not spec.get("file"):
            raise RuntimeError("rust program needs a file")
        return _compile_rust_file(HERE / spec["file"])
    if language == "typescript":
        if spec.get("file"):
            return _compile_typescript_file(HERE / spec["file"])
        raise RuntimeError("typescript program needs a file")
    source, filename = _load_program(spec)
    return driver.compile(source, filename)


def _compile_case(driver: Any, item: dict[str, Any], language: str) -> str:
    spec = _program_for(item, language)
    if "stored" in spec:
        raise RuntimeError(f"case {item['id']} is not a crash-resume program")
    return _compile_spec(driver, spec, language)


def _run_crash_resume(driver: Any, item: dict[str, Any], language: str) -> None:
    artifact_json = _compile_case(driver, item, language)
    children = None if language == "rust" else _child_artifacts(item)
    expected_status = "cancelled" if item.get("cancel") else "completed"
    started = driver.start(
        artifactJson=artifact_json,
        effects=item.get("effects"),
        eventPayload=item.get("event_payload"),
        autoDeliverEvent=item.get("auto_deliver_event", True),
        completionOrder=item.get("completion_order") or "source",
        childArtifacts=children,
        cancel=item.get("cancel", False),
    )
    assert started["status"] == expected_status, (
        f"{item['id']} uninterrupted status: {started}"
    )
    assert started["result"] == item.get("expected_result"), (
        f"{item['id']} uninterrupted result"
    )
    if item.get("expected_effect_log") is not None:
        assert driver.effect_log(started["handle"]) == item["expected_effect_log"]
    for crash_at in item.get("crash_ats") or []:
        handle = driver.crash_at(
            artifactJson=artifact_json,
            crashAt=crash_at,
            effects=item.get("effects"),
            eventPayload=item.get("event_payload"),
            autoDeliverEvent=item.get("auto_deliver_event", True),
            completionOrder=item.get("completion_order") or "source",
            childArtifacts=children,
            cancel=item.get("cancel", False),
        )
        resumed = driver.resume(
            handle=handle,
            effects=item.get("effects"),
            eventPayload=item.get("event_payload"),
            autoDeliverEvent=item.get("auto_deliver_event", True),
            completionOrder=item.get("completion_order") or "source",
            childArtifacts=children,
            cancel=item.get("cancel", False),
        )
        assert resumed["status"] == expected_status, (
            f"{item['id']} resume after {crash_at}: {resumed}"
        )
        assert resumed["result"] == item.get("expected_result"), (
            f"{item['id']} resume after {crash_at} result"
        )
        if item.get("expected_effect_log") is not None:
            assert driver.effect_log(handle) == item["expected_effect_log"]


def _run_pinning(driver: Any, item: dict[str, Any], language: str) -> None:
    spec = _program_for(item, language)
    stored = _compile_spec(driver, spec["stored"], language)
    other = _compile_spec(driver, spec["other"], language)
    stored_hash = json.loads(stored)["envelope"]["artifact_hash"]
    other_hash = json.loads(other)["envelope"]["artifact_hash"]
    assert stored_hash != other_hash, f"{item['id']} artifacts must differ"
    started = driver.start(artifactJson=stored, autoDeliverEvent=False)
    assert started["status"] == "suspended", f"{item['id']} start: {started}"
    continuation = driver.read_continuation(started["handle"])
    assert continuation["artifact_hash"] == stored_hash, f"{item['id']} pinned hash"
    try:
        driver.resume(
            handle=started["handle"],
            artifactJson=other,
            eventPayload="ok",
        )
    except Exception as exc:
        assert "artifact" in str(exc).lower(), (
            f"{item['id']} substitute must fail with artifact error, got {exc}"
        )
    else:
        raise AssertionError(f"{item['id']} substitute must fail")
    resumed = driver.resume(
        handle=started["handle"],
        eventPayload="ok",
    )
    assert resumed["status"] == "completed", (
        f"{item['id']} stored artifact resume: {resumed}"
    )


def run_kit(driver: Any) -> None:
    cases = load_cases()
    _run_reconstruct(driver, cases)
    for item in cases["semantic"]:
        kind = item.get("kind") or (
            "artifact-pinning" if item["id"] == "artifact-pinning" else "crash-resume"
        )
        for language in _languages(item, driver):
            if kind == "artifact-pinning":
                _run_pinning(driver, item, language)
            else:
                _run_crash_resume(driver, item, language)


if __name__ == "__main__":
    from drivers.python_sqlite import create_python_sqlite_driver

    run_kit(create_python_sqlite_driver())
    print("host-conformance-v1 ok")
