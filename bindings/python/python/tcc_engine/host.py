from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any, Optional

from tcc_engine._engine import EngineBinding
from tcc_engine.store import Store

FakeEffects = dict[str, Any]
EffectRunner = Callable[[str], Any]
CrashHook = Callable[[str, Optional[str]], None]


def map_effects(effects: FakeEffects) -> EffectRunner:
    def run(key: str) -> Any:
        if key not in effects:
            raise RuntimeError(f"no effect for `{key}`")
        return effects[key]

    return run


def _effect_runner(options: dict[str, Any]) -> EffectRunner:
    if options.get("run_effect") is not None:
        return options["run_effect"]
    return map_effects(options.get("effects") or {"generate": 42})


def _crash(options: dict[str, Any]) -> CrashHook:
    crash = options.get("crash")
    if crash is None:
        return lambda _hook, _detail=None: None
    return crash


def start_execution(
    *,
    db_path: str,
    artifact_json: str,
    execution_id: str = "first",
    run_effect: EffectRunner | None = None,
    effects: FakeEffects | None = None,
    event_payload: Any = None,
    owner_token: str = "owner-1",
    lease_ms: int = 60_000,
    budget: int = 256,
    auto_deliver_event: bool = True,
    effect_log_path: str | None = None,
    fail_counts: dict[str, int] | None = None,
    child_artifacts: dict[str, str] | None = None,
    cancel: bool = False,
    crash: CrashHook | None = None,
) -> dict[str, Any]:
    store = Store(db_path)
    try:
        import time

        envelope = json.loads(artifact_json)["envelope"]
        hash_ = envelope["artifact_hash"]
        store.put_artifact(hash_, artifact_json)
        store.create_execution(execution_id, hash_, owner_token, int(time.time() * 1000) + lease_ms)
        engine = EngineBinding(artifact_json, execution_id)
        return drive(
            store,
            engine,
            {
                "db_path": db_path,
                "artifact_json": artifact_json,
                "execution_id": execution_id,
                "owner_token": owner_token,
                "run_effect": run_effect,
                "effects": effects,
                "event_payload": event_payload,
                "budget": budget,
                "auto_deliver_event": auto_deliver_event,
                "effect_log_path": effect_log_path,
                "lease_ms": lease_ms,
                "fail_counts": fail_counts,
                "child_artifacts": child_artifacts,
                "cancel": cancel,
                "crash": crash,
            },
        )
    finally:
        store.close()


def resume_execution(
    *,
    db_path: str,
    artifact_json: str | None = None,
    execution_id: str = "first",
    run_effect: EffectRunner | None = None,
    effects: FakeEffects | None = None,
    event_payload: Any = None,
    owner_token: str = "owner-1",
    lease_ms: int = 60_000,
    budget: int = 256,
    auto_deliver_event: bool = True,
    effect_log_path: str | None = None,
    fail_counts: dict[str, int] | None = None,
    child_artifacts: dict[str, str] | None = None,
    cancel: bool = False,
    crash: CrashHook | None = None,
) -> dict[str, Any]:
    store = Store(db_path)
    try:
        import time

        now = int(time.time() * 1000)
        store.take_lease(execution_id, owner_token, now + lease_ms, now)
        execution = store.get_execution(execution_id)
        if execution is None:
            raise RuntimeError(f"unknown execution `{execution_id}`")
        saved = store.get_continuation(execution_id)
        artifact_json = artifact_json or store.get_artifact(execution["artifact_hash"])
        if not artifact_json:
            raise RuntimeError(f"missing artifact `{execution['artifact_hash']}`")
        if execution["status"] in {"completed", "cancelled", "failed"} and saved is not None:
            parsed = json.loads(saved["json"])
            return {
                "status": execution["status"],
                "result": parsed.get("result"),
                "continuationJson": saved["json"],
                "revision": saved["revision"],
            }
        engine = (
            EngineBinding.resume(artifact_json, saved["json"])
            if saved is not None
            else EngineBinding(artifact_json, execution_id)
        )
        return drive(
            store,
            engine,
            {
                "db_path": db_path,
                "artifact_json": artifact_json,
                "execution_id": execution_id,
                "owner_token": owner_token,
                "run_effect": run_effect,
                "effects": effects,
                "event_payload": event_payload,
                "budget": budget,
                "auto_deliver_event": auto_deliver_event,
                "effect_log_path": effect_log_path,
                "lease_ms": lease_ms,
                "fail_counts": fail_counts,
                "child_artifacts": child_artifacts,
                "cancel": cancel,
                "crash": crash,
            },
        )
    finally:
        store.close()


def drive(store: Store, engine: EngineBinding, options: dict[str, Any]) -> dict[str, Any]:
    run_effect = _effect_runner(options)
    budget = options.get("budget") or 256
    fail_counts = dict(options.get("fail_counts") or {})
    state = {"engine": engine}
    while True:
        outcome = json.loads(state["engine"].run_until_host(budget))
        next_step = handle_outcome(store, state, outcome, run_effect, fail_counts, options)
        if next_step != "continue":
            return next_step


def handle_outcome(
    store: Store,
    state: dict[str, EngineBinding],
    outcome: dict[str, Any],
    run_effect: EffectRunner,
    fail_counts: dict[str, int],
    options: dict[str, Any],
) -> Any:
    crash = _crash(options)
    kind = outcome.get("type")
    if kind == "completed":
        return snapshot(store, options["execution_id"], "completed", outcome.get("result"))
    if kind == "failed":
        return snapshot(store, options["execution_id"], "failed", outcome.get("message"))
    if kind == "cancelled":
        return snapshot(store, options["execution_id"], "cancelled", None)
    if kind == "budget_exhausted":
        raise RuntimeError("instruction budget exhausted")
    if kind == "suspended":
        return deliver_wake(store, state, options)
    if kind == "host":
        request = outcome["request"]
        rtype = request.get("type")
        if rtype == "run_effect":
            return execute_effect(store, state["engine"], request, run_effect, fail_counts, options)
        if rtype == "persist_effect":
            return persist_effect(store, state["engine"], request, options["execution_id"], crash)
        if rtype == "register_wait":
            return register_wait(store, state["engine"], request, options["execution_id"], crash)
        if rtype == "persist_checkpoint":
            return persist_checkpoint(store, state["engine"], request, options, crash)
        if rtype == "register_timer":
            store.upsert_timer(options["execution_id"], int(request.get("resume_at_ms") or 0))
            crash("after_register_timer", None)
            state["engine"].apply_response(json.dumps({"type": "ack"}))
            return "continue"
        if rtype == "create_child":
            store.upsert_child(
                str(request["invoke_id"]),
                options["execution_id"],
                str(request["child_execution_id"]),
                str(request["flow_name"]),
            )
            crash("after_create_child", None)
            state["engine"].apply_response(json.dumps({"type": "ack"}))
            return "continue"
        raise RuntimeError(f"unsupported host request `{rtype}`")
    raise RuntimeError("unknown engine outcome")


def execute_effect(
    store: Store,
    engine: EngineBinding,
    request: dict[str, Any],
    run_effect: EffectRunner,
    fail_counts: dict[str, int],
    options: dict[str, Any],
) -> str:
    crash = _crash(options)
    key = str(request["key"])
    idempotency_key = str(request["idempotency_key"])
    crash("before_effect_provider", None)
    existing = store.get_effect(options["execution_id"], key)
    if existing is not None and existing["status"] == "completed" and existing["result_json"]:
        engine.apply_response(json.dumps({"type": "effect_result", "value": json.loads(existing["result_json"])}))
        return "continue"
    store.mark_effect_started(options["execution_id"], key, idempotency_key)
    if fail_counts.get(key, 0) > 0:
        fail_counts[key] = fail_counts.get(key, 0) - 1
        if options.get("effect_log_path"):
            Path(options["effect_log_path"]).open("a", encoding="utf-8").write(f"{key}\n")
        store.fail_effect(
            options["execution_id"],
            key,
            idempotency_key,
            json.dumps({"t": "string", "v": "failed"}),
        )
        crash("after_persist_effect", key)
        return execute_effect(store, engine, request, run_effect, fail_counts, options)
    if options.get("effect_log_path"):
        Path(options["effect_log_path"]).open("a", encoding="utf-8").write(f"{key}\n")
    value = encode_value(run_effect(key))
    crash("after_effect_provider", None)
    engine.apply_response(json.dumps({"type": "effect_result", "value": value}))
    return "continue"


def persist_effect(
    store: Store, engine: EngineBinding, request: dict[str, Any], execution_id: str, crash: CrashHook
) -> str:
    crash("before_persist_effect", None)
    key = str(request["key"])
    idempotency_key = str(request.get("idempotency_key") or f"{execution_id}:{key}")
    result_json = json.dumps(request.get("result"))
    if request.get("status") == "failed":
        store.fail_effect(execution_id, key, idempotency_key, result_json)
    else:
        store.complete_effect(execution_id, key, idempotency_key, result_json)
    crash("after_persist_effect", key)
    engine.apply_response(json.dumps({"type": "ack"}))
    return "continue"


def register_wait(
    store: Store, engine: EngineBinding, request: dict[str, Any], execution_id: str, crash: CrashHook
) -> str:
    store.upsert_wait(str(request["wait_id"]), execution_id, str(request["event_name"]))
    crash("after_register_wait", None)
    engine.apply_response(json.dumps({"type": "ack"}))
    return "continue"


def persist_checkpoint(
    store: Store, engine: EngineBinding, request: dict[str, Any], options: dict[str, Any], crash: CrashHook
) -> str:
    crash("before_persist_checkpoint", None)
    revision = int(request["revision"])
    parsed = json.loads(engine.continuation_json())
    parsed["revision"] = revision
    json_text = json.dumps(parsed, separators=(",", ":"))
    store.commit_checkpoint(options["execution_id"], revision, json_text, parsed["status"], options["owner_token"])
    crash("after_persist_checkpoint", None)
    if parsed["status"] == "suspended":
        crash("after_wait_checkpoint", None)
    engine.apply_response(json.dumps({"type": "persist_confirmed", "revision": revision}))
    return "continue"


def deliver_wake(store: Store, state: dict[str, EngineBinding], options: dict[str, Any]) -> Any:
    crash = _crash(options)
    if options.get("cancel"):
        crash("before_cancel", None)
        state["engine"].apply_response(json.dumps({"type": "cancel"}))
        return "continue"
    continuation = json.loads(state["engine"].continuation_json())
    wait_kind = ((continuation.get("pending") or {}).get("kind") or {}).get("type")
    if wait_kind == "timer":
        timer = store.pending_timer(options["execution_id"])
        if timer is None or timer["status"] != "pending":
            return snapshot(store, options["execution_id"], "suspended", None)
        crash("before_timer_fired", None)
        store.resolve_timer(options["execution_id"])
        state["engine"].apply_response(json.dumps({"type": "timer_fired"}))
        return "continue"
    if wait_kind == "child":
        invoke_id = ((continuation.get("pending") or {}).get("kind") or {}).get("invoke_id")
        return deliver_child(store, state, options, invoke_id)
    return deliver_event(store, state["engine"], options)


def deliver_child(
    store: Store, state: dict[str, EngineBinding], options: dict[str, Any], invoke_id: str | None
) -> Any:
    crash = _crash(options)
    if not invoke_id:
        return snapshot(store, options["execution_id"], "suspended", None)
    child = store.get_child(invoke_id)
    if child is None:
        return snapshot(store, options["execution_id"], "suspended", None)
    if child["status"] == "completed" and child["result_json"]:
        crash("before_child_result", None)
        state["engine"].apply_response(
            json.dumps({"type": "child_result", "value": json.loads(child["result_json"])})
        )
        return "continue"
    artifact_json = (options.get("child_artifacts") or {}).get(child["flow_name"])
    if not artifact_json:
        raise RuntimeError(f"no child artifact for `{child['flow_name']}`")
    store.put_artifact(json.loads(artifact_json)["envelope"]["artifact_hash"], artifact_json)
    existing = store.get_execution(child["child_execution_id"])
    if existing is None:
        import time

        store.create_execution(
            child["child_execution_id"],
            json.loads(artifact_json)["envelope"]["artifact_hash"],
            options["owner_token"],
            int(time.time() * 1000) + (options.get("lease_ms") or 60_000),
        )
        child_engine = EngineBinding(artifact_json, child["child_execution_id"])
    else:
        saved = store.get_continuation(child["child_execution_id"])
        child_engine = (
            EngineBinding.resume(artifact_json, saved["json"])
            if saved is not None
            else EngineBinding(artifact_json, child["child_execution_id"])
        )
    child_result = drive(
        store,
        child_engine,
        {**options, "execution_id": child["child_execution_id"], "artifact_json": artifact_json, "cancel": False},
    )
    store.complete_child(invoke_id, json.dumps(child_result.get("result") or {"t": "undefined"}))
    crash("before_child_result", None)
    state["engine"].apply_response(json.dumps({"type": "child_result", "value": child_result.get("result")}))
    return "continue"


def deliver_event(store: Store, engine: EngineBinding, options: dict[str, Any]) -> Any:
    crash = _crash(options)
    wait = store.pending_wait(options["execution_id"])
    if wait is None or wait["status"] != "pending":
        return snapshot(store, options["execution_id"], "suspended", None)
    crash("before_event_payload", None)
    payload_json = None
    queued = store.take_event(options["execution_id"], wait["event_name"])
    if queued is not None:
        payload_json = queued["payload_json"]
    elif options.get("auto_deliver_event", True) and options.get("event_payload") is not None:
        payload_json = json.dumps(encode_value(options["event_payload"]))
    elif options.get("auto_deliver_event", True) and options.get("event_payload") is None:
        payload_json = json.dumps(encode_value("ok"))
    if payload_json is None:
        return snapshot(store, options["execution_id"], "suspended", None)
    store.resolve_wait(wait["wait_id"])
    crash("after_event_persist", None)
    engine.apply_response(json.dumps({"type": "event_payload", "value": json.loads(payload_json)}))
    return "continue"


def snapshot(store: Store, execution_id: str, status: str, result: Any) -> dict[str, Any]:
    saved = store.get_continuation(execution_id)
    return {
        "status": status,
        "result": result,
        "continuationJson": None if saved is None else saved["json"],
        "revision": 0 if saved is None else saved["revision"],
    }


def encode_value(value: Any) -> dict[str, Any]:
    if value is None:
        return {"t": "null"}
    if isinstance(value, bool):
        return {"t": "bool", "v": value}
    if isinstance(value, (int, float)):
        return {"t": "number", "v": float(value)}
    if isinstance(value, str):
        return {"t": "string", "v": value}
    if isinstance(value, list):
        return {"t": "array", "v": [encode_value(item) for item in value]}
    if isinstance(value, dict):
        return {"t": "object", "v": {key: encode_value(item) for key, item in value.items()}}
    raise RuntimeError("unsupported fake value")
