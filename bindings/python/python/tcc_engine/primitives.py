"""Compiler intrinsics for TCC durable operations. Compile with tcc_engine.compile; do not call at runtime."""

from __future__ import annotations

from typing import Any, Callable, NoReturn, TypeVar

T = TypeVar("T")


def _not_runtime() -> NoReturn:
    raise RuntimeError(
        "TCC durable operations are compiler intrinsics; compile the program instead of calling them at runtime"
    )


def effect(_key: str, _fn: Callable[[], T]) -> T:
    _not_runtime()


def wait_for_event(_name: str) -> Any:
    _not_runtime()


def sleep(_ms: float) -> None:
    _not_runtime()


def invoke(_name: str) -> Any:
    _not_runtime()
