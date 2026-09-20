#!/usr/bin/env python3
"""Install the local tcc-engine wheel in a fresh venv and run the first example."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
wheels = sorted((ROOT / "dist-packages").glob("tcc_engine-*.whl"))
if not wheels:
    raise SystemExit("no tcc_engine wheel in dist-packages/")
wheel = wheels[-1]

tmp = Path(tempfile.mkdtemp(prefix="tcc-wheel-verify-"))
venv = tmp / "venv"
subprocess.check_call([sys.executable, "-m", "venv", str(venv)])
pip = venv / "bin" / "pip"
python = venv / "bin" / "python"
env = {k: v for k, v in os.environ.items() if k != "CONDA_PREFIX"}
subprocess.check_call([str(pip), "install", str(wheel)], env=env)
code = r"""
from tcc_engine import PACKAGE_VERSION, LANGUAGE_SEMANTICS_VERSION, compile, start_execution
from tcc_engine.compile import artifact_json
import tempfile

assert PACKAGE_VERSION != LANGUAGE_SEMANTICS_VERSION
src = '''
from trigora import effect, wait_for_event

async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
'''
artifact = compile(src, filename="first.py")
db = tempfile.mkdtemp() + "/tcc.db"
result = start_execution(
    db_path=db,
    artifact_json=artifact_json(artifact),
    run_effect=lambda key: 42 if key == "generate" else (_ for _ in ()).throw(RuntimeError(key)),
)
assert result["status"] == "completed", result
assert result["result"]["v"]["result"]["v"] == 42
print("verified python wheel", PACKAGE_VERSION)
"""
subprocess.check_call([str(python), "-c", code], env=env)
print(f"verified {wheel.name} in {tmp}")
