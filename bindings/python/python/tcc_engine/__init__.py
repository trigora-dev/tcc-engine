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
    "EngineBinding",
    "Store",
    "artifact_json",
    "compile",
    "encode_value",
    "map_effects",
    "resume_execution",
    "start_execution",
]


def __getattr__(name: str):
    if name == "EngineBinding":
        from tcc_engine._engine import EngineBinding

        return EngineBinding
    if name == "Store":
        from tcc_engine.store import Store

        return Store
    if name in {"start_execution", "resume_execution", "encode_value", "map_effects"}:
        from tcc_engine import host

        return getattr(host, name)
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
