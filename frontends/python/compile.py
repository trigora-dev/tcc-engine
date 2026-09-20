"""TypeScript frontend sources now live in this tree for development tests.

The installable package is `tcc_engine` (see `bindings/python`).
"""

from tcc_engine.compile import (
    ENGINE_FORMAT_VERSION,
    FRONTEND_ID,
    FRONTEND_IDENTITY,
    FRONTEND_VERSION,
    LANGUAGE_SEMANTICS_VERSION,
    PACKAGE_VERSION,
    CompileError,
    artifact_json,
    compile,
)

__all__ = [
    "ENGINE_FORMAT_VERSION",
    "FRONTEND_ID",
    "FRONTEND_IDENTITY",
    "FRONTEND_VERSION",
    "LANGUAGE_SEMANTICS_VERSION",
    "PACKAGE_VERSION",
    "CompileError",
    "artifact_json",
    "compile",
]
