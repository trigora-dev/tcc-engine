#!/usr/bin/env python3
# Copyright (c) 2026 Trigora, Inc.
# SPDX-License-Identifier: BUSL-1.1
# See LICENSE for full terms.

"""Stage a self-contained tree so cibuildwheel can see the Cargo workspace.

The Python project lives in bindings/python and the crates live above it.
Linux copies the working directory into the container and requires the
package to sit inside that directory, so the wheel job runs cibuildwheel
from this copy. The copy stays outside the repository so it is not copied
into itself. pyproject.toml sits at the staged root and keeps the build
selection from bindings/python/pyproject.toml.
"""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SKIP = {".git", "target", "wheelhouse", "dist-packages", ".venv", "node_modules"}


def ignore(directory: str, names: list[str]) -> set[str]:
    del directory
    return {name for name in names if name in SKIP or name.endswith(".egg-info")}


def staged_project(source: str) -> str:
    staged = (
        source.replace(
            'manifest-path = "../../crates/tcc-python/Cargo.toml"',
            'manifest-path = "crates/tcc-python/Cargo.toml"',
        )
        .replace('python-source = "python"', 'python-source = "bindings/python/python"')
        .replace(
            'include = ["python/tcc_engine/_licenses/*", "python/tcc_engine/_licenses/**/*"]',
            'include = ["bindings/python/python/tcc_engine/_licenses/*", "bindings/python/python/tcc_engine/_licenses/**/*"]',
        )
        .replace(
            "bash ../../scripts/licenses/prepare-wheel.sh",
            "bash scripts/licenses/prepare-wheel.sh",
        )
    )
    if staged == source:
        raise SystemExit("the wheel project was not rewritten for the staged build")
    if 'build = "cp310-* cp311-* cp312-* cp313-* cp314-*"' not in staged:
        raise SystemExit("the staged build lost the CPython 3.10-3.14 selection")
    if 'skip = "pp* *-win32 *-musllinux_* cp3??t-*"' not in staged:
        raise SystemExit("the staged build lost the win32 and musllinux skip")
    if "bash ../../scripts/" in staged:
        raise SystemExit("the staged before-all still points outside the build")
    return staged


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: stage_wheel.py DEST")
    dest = Path(sys.argv[1]).resolve()
    if dest == ROOT or ROOT in dest.parents:
        raise SystemExit("stage destination must be outside the repository")
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(ROOT, dest, ignore=ignore)
    (dest / "pyproject.toml").write_text(
        staged_project((ROOT / "bindings/python/pyproject.toml").read_text())
    )
    required = [
        dest / "pyproject.toml",
        dest / "crates/tcc-python/Cargo.toml",
        dest / "Cargo.toml",
        dest / "Cargo.lock",
        dest / "scripts/licenses/prepare-wheel.sh",
        dest / "scripts/licenses/collect.py",
        dest / "scripts/licenses/about.toml",
        dest / "LICENSE",
        dest / "bindings/python/python/tcc_engine",
    ]
    for path in required:
        if not path.exists():
            raise SystemExit(f"staged wheel project is missing {path}")


if __name__ == "__main__":
    main()
