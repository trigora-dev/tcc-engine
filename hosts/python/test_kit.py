from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "conformance"))

from drivers.python_sqlite import create_python_sqlite_driver
from run import run_kit


def test_host_conformance_v1():
    run_kit(create_python_sqlite_driver())
