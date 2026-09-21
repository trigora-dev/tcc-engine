"""Host conformance v1 kit entry for the Python SQLite reference host."""

from __future__ import annotations

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from drivers.python_sqlite import create_python_sqlite_driver
from run import run_kit

if __name__ == "__main__":
    run_kit(create_python_sqlite_driver())
    print("host-conformance-v1 ok")
