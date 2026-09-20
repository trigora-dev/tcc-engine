"""Host conformance v1 kit entry for the Python reference host."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
raise SystemExit(pytest.main([str(ROOT / "hosts" / "python" / "test_kit.py"), "-q"]))
