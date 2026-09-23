"""Drive one ordinary-corpus artifact on the native Python engine binding."""

from __future__ import annotations

import json
import sys

from tcc_engine._engine import EngineBinding
from tcc_engine.compile import compile
from tcc_engine.canonical import canonical_stringify


def loads(text: str):
    return json.loads(text, parse_int=_parse_int)


def _parse_int(text: str):
    if text == "-0":
        return -0.0
    return int(text)


def drive(engine: EngineBinding, effects: dict, events: dict, child=None):
    for _ in range(10000):
        outcome = loads(engine.run_until_host(100000))
        kind = outcome["type"]
        if kind == "completed":
            return outcome["result"]
        if kind == "failed":
            raise RuntimeError(outcome.get("message", "failed"))
        if kind == "suspended":
            if child is not None:
                engine.apply_response(
                    json.dumps({"type": "child_result", "value": child})
                )
            elif events:
                name = next(iter(events))
                engine.apply_response(
                    json.dumps({"type": "event_payload", "value": events[name]})
                )
            else:
                raise RuntimeError("suspended without an event")
            continue
        if kind != "host":
            raise RuntimeError(f"unexpected outcome {kind}")
        request = outcome["request"]
        request_type = request["type"]
        if request_type == "run_effect":
            key = request["key"]
            if key not in effects:
                raise RuntimeError(f"missing effect {key}")
            engine.apply_response(
                json.dumps({"type": "effect_result", "value": effects[key]})
            )
        elif request_type == "persist_checkpoint":
            engine.apply_response(
                json.dumps(
                    {"type": "persist_confirmed", "revision": request["revision"]}
                )
            )
        elif request_type in {
            "persist_effect",
            "register_wait",
            "register_timer",
            "create_child",
        }:
            engine.apply_response(json.dumps({"type": "ack"}))
        else:
            raise RuntimeError(f"unexpected host request {request_type}")
    raise RuntimeError("drive did not finish")


def resume_once(artifact: str, args: list, effects: dict, events: dict, child=None) -> None:
    engine = EngineBinding(artifact, "ordinary-resume", canonical_stringify(args) if args else None)
    for _ in range(10000):
        outcome = loads(engine.run_until_host(100000))
        kind = outcome["type"]
        if kind == "suspended":
            continuation = loads(engine.continuation_json())
            if len(continuation.get("frames") or []) != 1:
                raise RuntimeError("resumed continuation has a helper frame")
            resumed = EngineBinding.resume(artifact, engine.continuation_json())
            if child is not None:
                resumed.apply_response(
                    json.dumps({"type": "child_result", "value": child})
                )
            else:
                name = next(iter(events))
                resumed.apply_response(
                    json.dumps({"type": "event_payload", "value": events[name]})
                )
            drive(resumed, effects, events, child)
            return
        if kind == "completed":
            return
        if kind != "host":
            raise RuntimeError(f"resume setup {kind}")
        request = outcome["request"]
        request_type = request["type"]
        if request_type == "persist_checkpoint":
            engine.apply_response(
                json.dumps(
                    {"type": "persist_confirmed", "revision": request["revision"]}
                )
            )
        elif request_type == "persist_effect" and not events:
            engine.apply_response(json.dumps({"type": "ack"}))
            continuation = loads(engine.continuation_json())
            if len(continuation.get("frames") or []) != 1:
                raise RuntimeError("resumed continuation has a helper frame")
            resumed = EngineBinding.resume(artifact, engine.continuation_json())
            drive(resumed, effects, events, child)
            return
        elif request_type in {"register_wait", "persist_effect", "create_child"}:
            engine.apply_response(json.dumps({"type": "ack"}))
        elif request_type == "run_effect":
            engine.apply_response(
                json.dumps(
                    {"type": "effect_result", "value": effects[request["key"]]}
                )
            )
        else:
            raise RuntimeError(f"unexpected {request_type} before resume")


def main() -> None:
    message = json.load(sys.stdin)
    mode = message["mode"]
    if mode == "compile":
        artifact = compile(message["source"], message["filename"])
        sys.stdout.write(canonical_stringify(artifact))
        return
    artifact = message["artifact"]
    args = message.get("args") or []
    effects = message.get("effects") or {}
    events = message.get("events") or {}
    child = message.get("child")
    engine = EngineBinding(artifact, "ordinary", canonical_stringify(args) if args else None)
    result = drive(engine, effects, events, child)
    if events or effects or child is not None:
        resume_once(artifact, args, effects, events, child)
    sys.stdout.write(canonical_stringify(result))


if __name__ == "__main__":
    try:
        main()
    except Exception as err:
        print(str(err), file=sys.stderr)
        sys.exit(1)
