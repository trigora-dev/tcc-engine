"""Drive a parent artifact and the child artifact it invokes.

The child is a separate artifact. The engine does not know which frontend produced it.
"""

from __future__ import annotations

import json
import sys

from tcc_engine._engine import EngineBinding
from tcc_engine.canonical import canonical_stringify
from tcc_engine.compile import compile


def loads(text: str):
    return json.loads(text)


def drive(engine: EngineBinding, child: str | None):
    child_result = None
    for _ in range(10000):
        outcome = loads(engine.run_until_host(100000))
        kind = outcome["type"]
        if kind == "completed":
            return outcome["result"]
        if kind == "failed":
            raise RuntimeError(outcome.get("message", "failed"))
        if kind == "suspended":
            if child_result is None:
                raise RuntimeError("suspended without a child result")
            engine.apply_response(json.dumps({"type": "child_result", "value": child_result}))
            continue
        if kind != "host":
            raise RuntimeError(f"unexpected outcome {kind}")
        request = outcome["request"]
        request_type = request["type"]
        if request_type == "persist_checkpoint":
            engine.apply_response(
                json.dumps({"type": "persist_confirmed", "revision": request["revision"]})
            )
        elif request_type == "create_child":
            if child is None:
                raise RuntimeError("child artifact missing")
            engine.apply_response(json.dumps({"type": "ack"}))
            args = request.get("args") or []
            child_engine = EngineBinding(
                child,
                "cross-language-child",
                canonical_stringify(args) if args else None,
            )
            child_result = drive(child_engine, None)
        elif request_type in {"persist_effect", "register_wait", "register_timer"}:
            engine.apply_response(json.dumps({"type": "ack"}))
        else:
            raise RuntimeError(f"unexpected host request {request_type}")
    raise RuntimeError("drive did not finish")


def main() -> None:
    message = json.load(sys.stdin)
    mode = message["mode"]
    if mode == "compile":
        artifact = compile(message["source"], message["filename"])
        sys.stdout.write(canonical_stringify(artifact))
        return
    result = drive(
        EngineBinding(message["parent"], "cross-language"),
        message["child"],
    )
    sys.stdout.write(canonical_stringify(result))


if __name__ == "__main__":
    try:
        main()
    except Exception as err:
        print(str(err), file=sys.stderr)
        sys.exit(1)
