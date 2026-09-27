#!/usr/bin/env python3
"""Fail when a packed TCC Engine artifact is missing its own license bundle."""

from __future__ import annotations

import sys
import tarfile
import zipfile
from pathlib import Path


def resolve(pattern: str) -> Path:
    path = Path(pattern)
    if path.is_file():
        return path
    matches = sorted(Path().glob(pattern))
    if len(matches) != 1:
        raise SystemExit(f"expected one archive for {pattern}, found {len(matches)}")
    return matches[0]


def npm_has(archive: Path, name: str) -> bool:
    with tarfile.open(archive) as tar:
        return any(item.endswith(name) for item in tar.getnames())


def wasm_package(archive: Path) -> None:
    with tarfile.open(archive) as tar:
        names = tar.getnames()
        notices = [name for name in names if name.endswith("THIRD_PARTY_LICENSES")]
        if len(notices) != 1:
            raise SystemExit("wasm package is missing THIRD_PARTY_LICENSES")
        text = tar.extractfile(notices[0]).read().decode()
        if "rusqlite" in text or "\nclap " in text or text.startswith("clap "):
            raise SystemExit("wasm notices include a Trigora CLI dependency")
        if not any(name.endswith("/LICENSE") or name.endswith("LICENSE") for name in names):
            raise SystemExit("wasm package is missing LICENSE")
    print("wasm compliance ok")


def wheel(archive: Path) -> None:
    with zipfile.ZipFile(archive) as packed:
        names = packed.namelist()
        notices = [name for name in names if name.endswith("THIRD_PARTY_LICENSES")]
        if not notices:
            raise SystemExit("wheel is missing THIRD_PARTY_LICENSES")
        text = packed.read(notices[0]).decode()
        if "pyo3" not in text:
            raise SystemExit("wheel notices are missing the python binding closure")
        if "rusqlite" in text or "clap " in text:
            raise SystemExit("wheel notices include a Trigora CLI dependency")
        if not any("third-party/" in name and name.endswith(".txt") for name in names):
            raise SystemExit("wheel is missing third-party license text")
    print("wheel compliance ok")


def frontend(archive: Path) -> None:
    if npm_has(archive, "THIRD_PARTY_LICENSES"):
        raise SystemExit("frontend package should not carry a binary notice bundle")
    print("frontend package has no binary notice bundle")


def main() -> None:
    kind, path = sys.argv[1], resolve(sys.argv[2])
    if kind == "wasm":
        wasm_package(path)
    elif kind == "wheel":
        wheel(path)
    elif kind == "frontend":
        frontend(path)
    else:
        raise SystemExit(f"unknown archive kind {kind}")


if __name__ == "__main__":
    main()
