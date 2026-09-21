import pytest

from tcc_engine.compile import (
    FRONTEND_IDENTITY,
    LANGUAGE_SEMANTICS_VERSION,
    PACKAGE_VERSION,
    CompileError,
    compile,
)

FIRST = """
from trigora import effect, wait_for_event

async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
"""


def test_compiles_the_first_example():
    artifact = compile(FIRST, filename="first.py")
    ops = [instruction["op"] for instruction in artifact["program"]["functions"][0]["instructions"]]
    assert artifact["program"]["functions"][0]["name"] == "run"
    assert ops == [
        "LoadConst",
        "Effect",
        "StoreLocal",
        "LoadConst",
        "WaitForEvent",
        "StoreLocal",
        "NewObject",
        "LoadLocal",
        "SetProp",
        "LoadLocal",
        "SetProp",
        "Return",
    ]
    assert len(artifact["envelope"]["artifact_hash"]) == 64
    assert "durable.effect" in artifact["envelope"]["required_engine_features"]
    assert artifact["envelope"]["frontend_id"] == FRONTEND_IDENTITY
    assert artifact["envelope"]["language_semantics_version"] == LANGUAGE_SEMANTICS_VERSION
    assert artifact["envelope"]["frontend_version"] == PACKAGE_VERSION
    assert PACKAGE_VERSION != LANGUAGE_SEMANTICS_VERSION


def test_repeat_compile_same_hash():
    first = compile(FIRST, filename="first.py")
    second = compile(FIRST, filename="first.py")
    assert first["envelope"]["artifact_hash"] == second["envelope"]["artifact_hash"]


def test_accepts_aliased_imports():
    source = """
from trigora import effect as durable_effect, wait_for_event as wait

async def run():
    result = await durable_effect("generate", lambda: 1)
    approval = await wait("approved")
    return {"result": result, "approval": approval}
"""
    artifact = compile(source)
    assert artifact["program"]["functions"][0]["instructions"][1]["op"] == "Effect"


def test_rejects_local_function_named_effect():
    source = """
async def run():
    async def effect(key, fn):
        return fn()
    result = await effect("generate", lambda: 1)
    return result
"""
    with pytest.raises(CompileError):
        compile(source)


def test_compile_errors_include_filename_and_span():
    with pytest.raises(CompileError) as err:
        compile("x = (", filename="bad.py")
    assert err.value.filename == "bad.py"
    assert err.value.span is not None
    assert err.value.span["file"] == "bad.py"
    assert err.value.span["start_line"] >= 1


def test_rejects_ctx_style_entry():
    source = """
async def run(ctx):
    await ctx.effect("generate", lambda: 1)
"""
    with pytest.raises(CompileError):
        compile(source)


def test_compiles_if_else():
    source = """
from trigora import effect

async def run():
    flag = await effect("flag", lambda: 1)
    if flag:
        result = await effect("taken", lambda: 42)
        return result
    else:
        result = await effect("skipped", lambda: 99)
        return result
"""
    artifact = compile(source)
    ops = [instruction["op"] for instruction in artifact["program"]["functions"][0]["instructions"]]
    assert "JumpIfFalse" in ops


def test_rejects_list_truthiness():
    source = """
from trigora import effect

async def run():
    flag = await effect("flag", lambda: 1)
    if []:
        return flag
    return flag
"""
    with pytest.raises(CompileError):
        compile(source)


def test_rejects_for():
    source = """
from trigora import effect

async def run():
    xs = await effect("xs", lambda: 1)
    for x in xs:
        return x
    return 0
"""
    with pytest.raises(CompileError) as err:
        compile(source)
    assert "for" in str(err.value)
    assert err.value.span is not None
    assert err.value.span["start_line"] >= 1
    assert err.value.why == "it cannot cross a durable checkpoint in the current Python subset"
    assert err.value.alternative == "use `while`"
    assert err.value.frontend_id == FRONTEND_IDENTITY
    assert err.value.frontend_version == PACKAGE_VERSION
    assert err.value.language_semantics_version == LANGUAGE_SEMANTICS_VERSION


FIRST_PRIMITIVES = """
from tcc_engine.primitives import effect, wait_for_event

async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
"""


def test_compiles_the_first_example_from_primitives():
    artifact = compile(FIRST_PRIMITIVES, filename="first.py")
    ops = [instruction["op"] for instruction in artifact["program"]["functions"][0]["instructions"]]
    assert ops[1] == "Effect"
    assert ops[4] == "WaitForEvent"


def test_primitives_and_trigora_imports_produce_the_same_hash():
    from_sdk = compile(FIRST, filename="first.py")
    from_primitives = compile(FIRST_PRIMITIVES, filename="first.py")
    assert from_sdk["envelope"]["artifact_hash"] == from_primitives["envelope"]["artifact_hash"]


def test_accepts_aliased_primitives_imports():
    source = """
from tcc_engine.primitives import effect as durable_effect, wait_for_event as wait

async def run():
    result = await durable_effect("generate", lambda: 1)
    approval = await wait("approved")
    return {"result": result, "approval": approval}
"""
    artifact = compile(source)
    assert artifact["program"]["functions"][0]["instructions"][1]["op"] == "Effect"


def test_rejects_imports_from_other_modules():
    source = """
from somewhere_else import effect

async def run():
    result = await effect("generate", lambda: 1)
    return result
"""
    with pytest.raises(CompileError):
        compile(source)


def test_primitives_are_not_runtime_callable():
    from tcc_engine.primitives import effect

    with pytest.raises(RuntimeError, match="compiler intrins"):
        effect("generate", lambda: 1)
