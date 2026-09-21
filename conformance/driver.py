"""Semantic host-conformance-v1 driver protocol."""

from __future__ import annotations

from typing import Any, Protocol


class ConformanceHandle(dict):
    execution_id: str
    workspace: str


class HostConformanceDriver(Protocol):
    language: str

    def compile(self, source: str, filename: str) -> str: ...
    def apply_delta(
        self, base: dict[str, Any], delta: dict[str, Any]
    ) -> dict[str, Any]: ...
    def start(self, **input: Any) -> dict[str, Any]: ...
    def crash_at(self, **input: Any) -> dict[str, Any]: ...
    def resume(self, **input: Any) -> dict[str, Any]: ...
    def read_continuation(self, handle: dict[str, Any]) -> dict[str, Any]: ...
    def effect_log(self, handle: dict[str, Any]) -> list[str]: ...
