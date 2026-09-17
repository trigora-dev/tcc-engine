from __future__ import annotations

import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "frontends" / "python"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from host import resume_execution, start_execution


def required(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"missing {name}")
    return value


mode = os.environ.get("TCC_MODE", "start")
db_path = required("TCC_DB_PATH")
execution_id = os.environ.get("TCC_EXECUTION_ID", "first")
owner_token = os.environ.get("TCC_OWNER_TOKEN", "owner-1")
result_path = os.environ.get("TCC_RESULT_PATH")
effect_log_path = os.environ.get("TCC_EFFECT_LOG_PATH")
config = {}
if os.environ.get("TCC_CONFIG_PATH"):
    config = json.loads(Path(os.environ["TCC_CONFIG_PATH"]).read_text(encoding="utf-8"))
event_payload = json.loads(os.environ["TCC_EVENT_PAYLOAD"]) if os.environ.get("TCC_EVENT_PAYLOAD") else config.get("event_payload")
auto_deliver_event = (
    os.environ.get("TCC_AUTO_EVENT") != "0"
    if os.environ.get("TCC_AUTO_EVENT") is not None
    else config.get("auto_deliver_event", True)
)
effects = json.loads(os.environ["TCC_EFFECTS"]) if os.environ.get("TCC_EFFECTS") else config.get("effects", {"generate": 42})

options = {
    "db_path": db_path,
    "execution_id": execution_id,
    "owner_token": owner_token,
    "event_payload": event_payload,
    "auto_deliver_event": auto_deliver_event,
    "effect_log_path": effect_log_path,
    "effects": effects,
    "fail_counts": config.get("fail_counts"),
    "child_artifacts": config.get("child_artifacts"),
    "cancel": config.get("cancel", os.environ.get("TCC_CANCEL") == "1"),
}

result = (
    resume_execution(**options)
    if mode == "resume"
    else start_execution(artifact_json=required("TCC_ARTIFACT_JSON"), **options)
)
text = json.dumps(result)
if result_path:
    Path(result_path).write_text(text, encoding="utf-8")
sys.stdout.write(text + "\n")
