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
_UNSET = object()


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
    program_args: Any = _UNSET,
    run_effect: EffectRunner | None = None,
    effects: FakeEffects | None = None,
    event_payload: Any = None,
    owner_token: str = "owner-1",
    lease_ms: int = 60_000,
    budget: int = 256,
    auto_deliver_event: bool = True,
    completion_order: str = "source",
    effect_log_path: str | None = None,
    fail_counts: dict[str, int] | None = None,
    child_artifacts: dict[str, str] | None = None,
    cancel: bool = False,
    crash: CrashHook | None = None,
    persist: str | None = None,
    on_event: Any | None = None,
    packing: str | None = None,
    min_full_bytes: float | None = None,
    max_delta_ratio: float | None = None,
) -> dict[str, Any]:
    store = Store(
        db_path,
        persist,
        on_event,
        packing=packing,
        min_full_bytes=min_full_bytes,
        max_delta_ratio=max_delta_ratio,
    )
    try:
        return run_on_store(
            store,
            artifact_json=artifact_json,
            execution_id=execution_id,
            run_effect=run_effect,
            effects=effects,
            event_payload=event_payload,
            owner_token=owner_token,
            lease_ms=lease_ms,
            budget=budget,
            auto_deliver_event=auto_deliver_event,
            completion_order=completion_order,
            effect_log_path=effect_log_path,
            fail_counts=fail_counts,
            child_artifacts=child_artifacts,
            cancel=cancel,
            crash=crash,
            db_path=db_path,
            program_args=program_args,
        )
    finally:
        store.close()


def run_on_store(
    store: Store,
    *,
    artifact_json: str,
    execution_id: str = "first",
    run_effect: EffectRunner | None = None,
    effects: FakeEffects | None = None,
    event_payload: Any = None,
    owner_token: str = "owner-1",
    lease_ms: int = 60_000,
    budget: int = 256,
    auto_deliver_event: bool = True,
    completion_order: str = "source",
    effect_log_path: str | None = None,
    fail_counts: dict[str, int] | None = None,
    child_artifacts: dict[str, str] | None = None,
    cancel: bool = False,
    crash: CrashHook | None = None,
    db_path: str = "",
    program_args: Any = _UNSET,
) -> dict[str, Any]:
    if store.is_grouping():
        raise RuntimeError("run_on_store cannot acknowledge checkpoints inside an open group; use run_batch_on_store")
    engine, options = _prepare_on_store(
        store,
        artifact_json=artifact_json,
        execution_id=execution_id,
        run_effect=run_effect,
        effects=effects,
        event_payload=event_payload,
        owner_token=owner_token,
        lease_ms=lease_ms,
        budget=budget,
        auto_deliver_event=auto_deliver_event,
        completion_order=completion_order,
        effect_log_path=effect_log_path,
        fail_counts=fail_counts,
        child_artifacts=child_artifacts,
        cancel=cancel,
        crash=crash,
        db_path=db_path,
        program_args=program_args,
    )
    return drive(store, engine, options)


def _prepare_on_store(store: Store, **options: Any) -> tuple[EngineBinding, dict[str, Any]]:
    import time

    artifact_json = options["artifact_json"]
    execution_id = options.get("execution_id", "first")
    owner_token = options.get("owner_token", "owner-1")
    lease_ms = options.get("lease_ms", 60_000)
    envelope = json.loads(artifact_json)["envelope"]
    hash_ = envelope["artifact_hash"]
    store.put_artifact(hash_, artifact_json)
    if store.get_execution(execution_id) is None:
        store.create_execution(execution_id, hash_, owner_token, int(time.time() * 1000) + lease_ms)
    args_json = _program_args_json(options)
    engine = (
        EngineBinding(artifact_json, execution_id)
        if args_json is None
        else EngineBinding(artifact_json, execution_id, args_json)
    )
    return engine, {**options, "execution_id": execution_id, "owner_token": owner_token, "fail_counts": dict(options.get("fail_counts") or {})}


def run_batch_on_store(
    store: Store,
    inputs: list[dict[str, Any]],
    before_commit: Callable[[], None] | None = None,
    after_commit: Callable[[], None] | None = None,
) -> list[dict[str, Any]]:
    """Advance one checkpoint per execution per round; confirm only after the shared commit."""
    if store.is_grouping():
        raise RuntimeError("batch runner requires a store without an open group")
    sessions = [_prepare_on_store(store, **item) for item in inputs]
    results: list[dict[str, Any] | None] = [None] * len(inputs)
    parent_count = len(inputs)
    while any(result is None for result in results):
        queued: list[tuple[int, int]] = []
        store.begin_group()
        try:
            for i, (engine, options) in enumerate(sessions):
                if results[i] is not None:
                    continue
                result = drive(store, engine, options, defer_checkpoint=True)
                if isinstance(result, dict) and "queuedRevision" in result:
                    queued.append((i, result["queuedRevision"]))
                elif not (isinstance(result, dict) and result.get("activateChild")):
                    results[i] = result
            if queued and before_commit:
                before_commit()
            store.end_group()
        except Exception:
            store.abort_group()
            raise
        if queued and after_commit:
            after_commit()
        for i, revision in queued:
            engine = sessions[i][0]
            engine.apply_response(json.dumps({"type": "persist_confirmed", "revision": revision}))
        for created in store.take_created_children():
            child_id = created["child_execution_id"]
            if any(options.get("execution_id") == child_id for _, options in sessions):
                continue
            parent = next(
                (options for _, options in sessions if options.get("execution_id") == created["parent_execution_id"]),
                None,
            )
            artifact_json = (parent or {}).get("child_artifacts", {}).get(created["flow_name"]) if parent else None
            if not artifact_json:
                raise RuntimeError(f"no child artifact for `{created['flow_name']}`")
            child_options = {
                key: value
                for key, value in parent.items()
                if key not in {"program_args", "program_args_json"}
            }
            row = store.get_child_by_execution_id(child_id)
            if row is not None and row["args_json"] is not None:
                child_options["program_args_json"] = row["args_json"]
            sessions.append(
                _prepare_on_store(
                    store,
                    **{
                        **child_options,
                        "execution_id": child_id,
                        "artifact_json": artifact_json,
                        "cancel": False,
                    },
                )
            )
            results.append(None)
    return [result for result in results[:parent_count] if result is not None]


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
    completion_order: str = "source",
    effect_log_path: str | None = None,
    fail_counts: dict[str, int] | None = None,
    child_artifacts: dict[str, str] | None = None,
    cancel: bool = False,
    crash: CrashHook | None = None,
    persist: str | None = None,
    on_event: Any | None = None,
    packing: str | None = None,
    min_full_bytes: float | None = None,
    max_delta_ratio: float | None = None,
) -> dict[str, Any]:
    store = Store(
        db_path,
        persist,
        on_event,
        packing=packing,
        min_full_bytes=min_full_bytes,
        max_delta_ratio=max_delta_ratio,
    )
    try:
        import time

        now = int(time.time() * 1000)
        store.take_lease(execution_id, owner_token, now + lease_ms, now)
        execution = store.get_execution(execution_id)
        if execution is None:
            raise RuntimeError(f"unknown execution `{execution_id}`")
        saved = store.get_continuation(execution_id, observe_restore=True)
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
                "completion_order": completion_order,
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


def drive(store: Store, engine: EngineBinding, options: dict[str, Any], defer_checkpoint: bool = False) -> Any:
    if store.is_grouping() and not defer_checkpoint:
        raise RuntimeError("single-execution drive cannot run inside an open checkpoint group")
    run_effect = _effect_runner(options)
    budget = options.get("budget") or 256
    if options.get("fail_counts") is None:
        options["fail_counts"] = {}
    fail_counts = options["fail_counts"]
    state = {"engine": engine}
    while True:
        outcome = json.loads(state["engine"].run_until_host(budget))
        next_step = handle_outcome(store, state, outcome, run_effect, fail_counts, options, defer_checkpoint)
        if next_step != "continue":
            return next_step


def handle_outcome(
    store: Store,
    state: dict[str, EngineBinding],
    outcome: dict[str, Any],
    run_effect: EffectRunner,
    fail_counts: dict[str, int],
    options: dict[str, Any],
    defer_checkpoint: bool = False,
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
        return deliver_wake(store, state, options, defer_checkpoint)
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
            return persist_checkpoint(store, state["engine"], request, options, crash, defer_checkpoint)
        if rtype == "register_timer":
            store.upsert_timer(
                options["execution_id"],
                int(request.get("resume_at_ms") or 0),
                request.get("branch") or "",
            )
            crash("after_register_timer", None)
            state["engine"].apply_response(json.dumps({"type": "ack"}))
            return "continue"
        if rtype == "create_child":
            return enqueue_child(store, state["engine"], request, options, crash)
        store.observe({"type": "runtime.error", "message": f"unsupported host request `{rtype}`"})
        raise RuntimeError(f"unsupported host request `{rtype}`")
    store.observe({"type": "runtime.error", "message": "unknown engine outcome"})
    raise RuntimeError("unknown engine outcome")


def _program_args_json(options: dict[str, Any]) -> str | None:
    stored = options.get("program_args_json")
    if stored is not None:
        return None if stored == "[]" else stored
    if options.get("program_args", _UNSET) is _UNSET:
        return None
    args = options["program_args"]
    if not isinstance(args, list):
        raise TypeError("program_args must be a list")
    if len(args) == 0:
        return None
    return json.dumps([encode_value(item) for item in args], separators=(",", ":"))


def _child_args_json(request: dict[str, Any]) -> str | None:
    if "args" not in request:
        return None
    args = request["args"]
    if not isinstance(args, list):
        raise RuntimeError("create_child args must be an array")
    if len(args) == 0:
        return None
    return json.dumps(args, separators=(",", ":"))


def _engine_for_child(artifact_json: str, execution_id: str, args_json: str | None) -> EngineBinding:
    if args_json is None:
        return EngineBinding(artifact_json, execution_id)
    return EngineBinding(artifact_json, execution_id, args_json)


def enqueue_child(
    store: Store,
    engine: EngineBinding,
    request: dict[str, Any],
    options: dict[str, Any],
    crash: CrashHook,
) -> str:
    program_name = str(request["program_name"])
    artifact_json = (options.get("child_artifacts") or {}).get(program_name)
    if not artifact_json:
        store.observe(
            {
                "type": "runtime.error",
                "executionId": options["execution_id"],
                "message": f"no child artifact for `{program_name}`",
            }
        )
        raise RuntimeError(f"no child artifact for `{program_name}`")
    artifact_hash = json.loads(artifact_json)["envelope"]["artifact_hash"]
    store.put_artifact(artifact_hash, artifact_json)
    import time

    store.enqueue_create_child(
        {
            "invoke_id": str(request["invoke_id"]),
            "parent_execution_id": options["execution_id"],
            "child_execution_id": str(request["child_execution_id"]),
            "flow_name": program_name,
            "artifact_hash": artifact_hash,
            "owner_token": options["owner_token"],
            "lease_until": int(time.time() * 1000) + (options.get("lease_ms") or 60_000),
            "args_json": _child_args_json(request),
        }
    )
    crash("after_create_child", None)
    engine.apply_response(json.dumps({"type": "ack"}))
    return "continue"


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
        store.observe({"type": "effect.journal_hit", "executionId": options["execution_id"]})
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
    produced = run_effect(key)
    if produced == "__fail__":
        engine.apply_response(json.dumps({"type": "effect_failed", "message": "failed"}))
        return "continue"
    value = encode_value(produced)
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
    store: Store, engine: EngineBinding, request: dict[str, Any], options: dict[str, Any], crash: CrashHook,
    defer_checkpoint: bool = False,
) -> Any:
    crash("before_persist_checkpoint", None)
    revision = int(request["revision"])
    if store.persist == "replay":
        current = store.get_execution(options["execution_id"])
        if current is not None and current["revision"] >= revision:
            engine.apply_response(json.dumps({"type": "persist_confirmed", "revision": revision}))
            return "continue"
    parsed = json.loads(engine.continuation_json())
    parsed["revision"] = revision
    json_text = json.dumps(parsed, separators=(",", ":"))
    kind = "delta" if request.get("kind") == "delta" else "snapshot"
    delta = request.get("delta")
    store.enqueue_checkpoint(
        options["execution_id"],
        revision,
        json_text,
        parsed["status"],
        options["owner_token"],
        kind=kind,
        delta_json=None if delta is None else json.dumps(delta, separators=(",", ":")),
        materialize=bool(request.get("materialize")) or kind == "snapshot",
    )
    if parsed["status"] == "completed":
        related = store.get_child_by_execution_id(options["execution_id"])
        if related is not None:
            store.enqueue_complete_child(
                related["invoke_id"],
                json.dumps(parsed.get("result") or {"t": "undefined"}, separators=(",", ":")),
            )
    if defer_checkpoint:
        return {"queuedRevision": revision}
    store.flush_if_ungrouped()
    crash("after_persist_checkpoint", None)
    if parsed["status"] == "suspended":
        crash("after_wait_checkpoint", None)
    engine.apply_response(json.dumps({"type": "persist_confirmed", "revision": revision}))
    return "continue"


def deliver_wake(
    store: Store, state: dict[str, EngineBinding], options: dict[str, Any], defer_checkpoint: bool = False
) -> Any:
    crash = _crash(options)
    if options.get("cancel"):
        crash("before_cancel", None)
        state["engine"].apply_response(json.dumps({"type": "cancel"}))
        return "continue"
    continuation = json.loads(state["engine"].continuation_json())
    join = continuation.get("join")
    if join and join.get("state") == "active" and not continuation.get("pending"):
        return deliver_join(store, state, options, join, defer_checkpoint)
    wait_kind = ((continuation.get("pending") or {}).get("kind") or {}).get("type")
    if wait_kind == "timer":
        timer = store.pending_timer(options["execution_id"])
        if timer is None or timer["status"] != "pending":
            return snapshot(store, options["execution_id"], "suspended", None)
        crash("before_timer_fired", None)
        branch_id = timer["branch_id"] or ""
        store.resolve_timer(options["execution_id"], branch_id)
        payload = {"type": "timer_fired"}
        if branch_id:
            payload["branch"] = branch_id
        state["engine"].apply_response(json.dumps(payload))
        return "continue"
    if wait_kind == "child":
        invoke_id = ((continuation.get("pending") or {}).get("kind") or {}).get("invoke_id")
        return deliver_child(store, state, options, invoke_id, defer_checkpoint)
    return deliver_event(store, state["engine"], options)


def deliver_join(
    store: Store,
    state: dict[str, EngineBinding],
    options: dict[str, Any],
    join: dict[str, Any],
    defer_checkpoint: bool = False,
) -> Any:
    branches = [branch for branch in (join.get("branches") or []) if branch.get("phase") == "registered"]
    branches.sort(key=lambda branch: branch.get("index", 0))
    if options.get("completion_order") == "reverse":
        branches.reverse()
    if not branches:
        return snapshot(store, options["execution_id"], "suspended", None)
    branch = branches[0]
    op = (branch.get("op") or {}).get("type")
    branch_id = branch["branch_id"]
    crash = _crash(options)
    if op == "timer":
        timer = store.pending_timer(options["execution_id"], branch_id)
        if timer is None or timer["status"] != "pending":
            return snapshot(store, options["execution_id"], "suspended", None)
        crash("before_timer_fired", None)
        store.resolve_timer(options["execution_id"], branch_id)
        state["engine"].apply_response(json.dumps({"type": "timer_fired", "branch": branch_id}))
        return "continue"
    if op == "child":
        return deliver_child(store, state, options, branch_id, defer_checkpoint, branch_id)
    return deliver_event(store, state["engine"], options, branch_id)


def deliver_child(
    store: Store,
    state: dict[str, EngineBinding],
    options: dict[str, Any],
    invoke_id: str | None,
    defer_checkpoint: bool = False,
    branch: str | None = None,
) -> Any:
    crash = _crash(options)
    if not invoke_id:
        return snapshot(store, options["execution_id"], "suspended", None)
    child = store.get_child(invoke_id)
    if child is None:
        return snapshot(store, options["execution_id"], "suspended", None)
    if child["status"] == "completed" and child["result_json"]:
        crash("before_child_result", None)
        payload = {"type": "child_result", "value": json.loads(child["result_json"])}
        if branch:
            payload["branch"] = branch
        state["engine"].apply_response(json.dumps(payload))
        return "continue"
    if store.is_grouping() or defer_checkpoint:
        return {"activateChild": True}
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
        child_engine = _engine_for_child(artifact_json, child["child_execution_id"], child["args_json"])
    else:
        saved = store.get_continuation(child["child_execution_id"])
        child_engine = (
            EngineBinding.resume(artifact_json, saved["json"])
            if saved is not None
            else _engine_for_child(artifact_json, child["child_execution_id"], child["args_json"])
        )
    child_result = drive(
        store,
        child_engine,
        {**options, "execution_id": child["child_execution_id"], "artifact_json": artifact_json, "cancel": False},
    )
    store.complete_child(invoke_id, json.dumps(child_result.get("result") or {"t": "undefined"}))
    crash("before_child_result", None)
    payload = {"type": "child_result", "value": child_result.get("result")}
    if branch:
        payload["branch"] = branch
    state["engine"].apply_response(json.dumps(payload))
    return "continue"


def deliver_event(store: Store, engine: EngineBinding, options: dict[str, Any], wait_id: str | None = None) -> Any:
    crash = _crash(options)
    wait = store.get_wait(wait_id) if wait_id else store.pending_wait(options["execution_id"])
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
    payload = {"type": "event_payload", "value": json.loads(payload_json)}
    if wait_id:
        payload["branch"] = wait_id
    engine.apply_response(json.dumps(payload))
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
