"""Compile a supported Python subset into a TCC artifact."""

from __future__ import annotations

import ast
import hashlib
import math
from typing import Any

from canonical import canonical_stringify

ENGINE_FORMAT_VERSION = 1
FRONTEND_ID = "python"
FRONTEND_VERSION = "0.0.0"
LANGUAGE_SEMANTICS_VERSION = "py.subset.v1"
SDK_MODULE = "trigora"
DURABLE = {"effect", "wait_for_event", "sleep", "invoke"}


class CompileError(Exception):
    pass


def compile(source: str, filename: str = "input.py") -> dict[str, Any]:
    try:
        tree = ast.parse(source, filename=filename)
    except SyntaxError as err:
        raise CompileError(f"{filename}: {err.msg}") from err
    aliases = collect_imports(tree, filename)
    entry = find_entry(tree)
    function = lower_function(entry, aliases, filename)
    engine, host = required_from(function["instructions"])
    artifact = {
        "envelope": {
            "artifact_hash": "",
            "frontend_id": FRONTEND_ID,
            "frontend_version": FRONTEND_VERSION,
            "language_semantics_version": LANGUAGE_SEMANTICS_VERSION,
            "engine_format_version": ENGINE_FORMAT_VERSION,
            "required_engine_features": engine,
            "required_host_capabilities": host,
            "runtime_modules": [],
        },
        "program": {"entry": 0, "functions": [function]},
    }
    artifact["envelope"]["artifact_hash"] = hash_artifact(artifact)
    return artifact


def artifact_json(artifact: dict[str, Any]) -> str:
    return canonical_stringify(artifact)


def hash_artifact(artifact: dict[str, Any]) -> str:
    copy = {
        "envelope": {**artifact["envelope"], "artifact_hash": ""},
        "program": artifact["program"],
    }
    return hashlib.sha256(canonical_stringify(copy).encode("utf-8")).hexdigest()


def collect_imports(tree: ast.Module, filename: str) -> dict[str, str]:
    aliases: dict[str, str] = {}
    for statement in tree.body:
        if isinstance(statement, ast.ImportFrom):
            if statement.module != SDK_MODULE or statement.level:
                raise CompileError(f"unsupported import from `{statement.module or '?'}`")
            for alias in statement.names:
                if alias.name not in DURABLE:
                    raise CompileError(f"unknown durable import `{alias.name}`")
                aliases[alias.asname or alias.name] = alias.name
            continue
        if isinstance(statement, ast.Import):
            raise CompileError("unsupported import")
    return aliases


def find_entry(tree: ast.Module) -> ast.AsyncFunctionDef:
    entry: ast.AsyncFunctionDef | None = None
    for statement in tree.body:
        if isinstance(statement, ast.ImportFrom):
            continue
        if isinstance(statement, ast.AsyncFunctionDef) and statement.name == "run":
            if entry is not None:
                raise CompileError("multiple `async def run` entry points")
            if statement.args.args or statement.args.posonlyargs or statement.args.kwonlyargs:
                raise CompileError("entry function must not take parameters")
            if statement.args.vararg or statement.args.kwarg:
                raise CompileError("entry function must not take parameters")
            entry = statement
            continue
        raise CompileError(f"unsupported top-level statement: {type(statement).__name__}")
    if entry is None:
        raise CompileError("missing `async def run` entry point")
    return entry


def lower_function(
    entry: ast.AsyncFunctionDef, aliases: dict[str, str], filename: str
) -> dict[str, Any]:
    lower = Lowerer(aliases, filename)
    lower.push_scope()
    for statement in entry.body:
        lower.statement(statement)
    lower.pop_scope()
    lower.seal()
    return {
        "id": 0,
        "name": entry.name,
        "param_count": 0,
        "local_count": lower.max_slots,
        "instructions": lower.instructions,
        "spans": lower.spans,
    }


class Loop:
    def __init__(self) -> None:
        self.breaks: list[int] = []
        self.continues: list[int] = []
        self.continue_target: int | None = None


class Lowerer:
    def __init__(self, aliases: dict[str, str], filename: str) -> None:
        self.aliases = aliases
        self.filename = filename
        self.instructions: list[dict[str, Any]] = []
        self.spans: list[dict[str, Any] | None] = []
        self.scopes: list[dict[str, int]] = []
        self.slots: dict[str, int] = {}
        self.loops: list[Loop] = []
        self.max_slots = 0

    def pc(self) -> int:
        return len(self.instructions)

    def emit(self, instruction: dict[str, Any], node: ast.AST) -> int:
        index = len(self.instructions)
        self.instructions.append(instruction)
        self.spans.append(span_of(self.filename, node))
        return index

    def patch(self, index: int, target: int) -> None:
        instruction = self.instructions[index]
        if "target" not in instruction:
            raise CompileError("internal: patch target missing")
        instruction["target"] = target

    def push_scope(self) -> None:
        self.scopes.append({})

    def pop_scope(self) -> None:
        if not self.scopes:
            raise CompileError("internal: scope underflow")
        self.scopes.pop()

    def alloc(self) -> int:
        slot = self.max_slots
        self.max_slots += 1
        return slot

    def declare(self, name: str) -> int:
        scope = self.scopes[-1]
        if name in scope:
            raise CompileError(f"duplicate binding `{name}`")
        if name in self.slots:
            slot = self.slots[name]
        else:
            slot = self.alloc()
            self.slots[name] = slot
        scope[name] = slot
        return slot

    def lookup(self, name: str) -> int:
        if name not in self.slots:
            raise CompileError(f"unknown local `{name}`")
        return self.slots[name]

    def bind_or_assign(self, name: str) -> int:
        if name in self.slots:
            return self.slots[name]
        return self.declare(name)

    def statement(self, statement: ast.stmt) -> None:
        if isinstance(statement, ast.Pass):
            return
        if isinstance(statement, ast.Expr):
            self.expression(statement.value)
            self.emit({"op": "Pop"}, statement)
            return
        if isinstance(statement, ast.Assign):
            self.assign(statement)
            return
        if isinstance(statement, ast.AnnAssign):
            raise CompileError("annotated assignment is not supported")
        if isinstance(statement, ast.AugAssign):
            raise CompileError("augmented assignment is not supported")
        if isinstance(statement, ast.Return):
            if statement.value is None:
                self.emit({"op": "LoadConst", "value": {"t": "null"}}, statement)
            else:
                self.expression(statement.value)
            self.emit({"op": "Return"}, statement)
            return
        if isinstance(statement, ast.If):
            self.if_statement(statement)
            return
        if isinstance(statement, ast.While):
            self.while_statement(statement)
            return
        if isinstance(statement, ast.Break):
            if not self.loops:
                raise CompileError("`break` outside a loop")
            self.loops[-1].breaks.append(self.emit({"op": "Jump", "target": 0}, statement))
            return
        if isinstance(statement, ast.Continue):
            if not self.loops:
                raise CompileError("`continue` outside a loop")
            loop = self.loops[-1]
            if loop.continue_target is not None:
                self.emit({"op": "Jump", "target": loop.continue_target}, statement)
            else:
                loop.continues.append(self.emit({"op": "Jump", "target": 0}, statement))
            return
        if isinstance(statement, ast.Raise):
            self.raise_statement(statement)
            return
        if isinstance(statement, ast.Try):
            self.try_statement(statement)
            return
        if isinstance(statement, ast.For):
            raise CompileError("`for` is not supported; use `while`")
        if isinstance(statement, ast.FunctionDef) or isinstance(statement, ast.AsyncFunctionDef):
            raise CompileError("nested functions are not supported")
        if isinstance(statement, ast.ClassDef):
            raise CompileError("classes are not supported")
        raise CompileError(f"unsupported statement: {type(statement).__name__}")

    def assign(self, statement: ast.Assign) -> None:
        if len(statement.targets) != 1 or not isinstance(statement.targets[0], ast.Name):
            raise CompileError("assignment target must be a local")
        name = statement.targets[0].id
        slot = self.bind_or_assign(name)
        self.expression(statement.value)
        self.emit({"op": "StoreLocal", "local": slot}, statement)

    def if_statement(self, statement: ast.If) -> None:
        self.condition(statement.test)
        jump_false = self.emit({"op": "JumpIfFalse", "target": 0}, statement.test)
        self.block(statement.body)
        if statement.orelse:
            jump_end = self.emit({"op": "Jump", "target": 0}, statement)
            self.patch(jump_false, self.pc())
            if len(statement.orelse) == 1 and isinstance(statement.orelse[0], ast.If):
                self.if_statement(statement.orelse[0])
            else:
                self.block(statement.orelse)
            self.patch(jump_end, self.pc())
        else:
            self.patch(jump_false, self.pc())

    def while_statement(self, statement: ast.While) -> None:
        if statement.orelse:
            raise CompileError("`while/else` is not supported")
        start = self.pc()
        loop = Loop()
        loop.continue_target = start
        self.loops.append(loop)
        self.condition(statement.test)
        jump_false = self.emit({"op": "JumpIfFalse", "target": 0}, statement.test)
        self.block(statement.body)
        self.emit({"op": "Jump", "target": start}, statement)
        end = self.pc()
        self.patch(jump_false, end)
        for index in loop.breaks:
            self.patch(index, end)
        self.loops.pop()

    def try_statement(self, statement: ast.Try) -> None:
        if statement.orelse:
            raise CompileError("`try/else` is not supported")
        if not statement.handlers and not statement.finalbody:
            raise CompileError("try requires except or finally")
        push = self.emit({"op": "PushTry", "catch": 0, "finally": None}, statement)
        self.block(statement.body)
        self.emit({"op": "PopTry"}, statement)
        jump_join = self.emit({"op": "Jump", "target": 0}, statement)
        handler_pc = self.pc()
        self.instructions[push]["catch"] = handler_pc
        if statement.handlers:
            if len(statement.handlers) != 1:
                raise CompileError("one except clause is supported")
            handler = statement.handlers[0]
            self.push_scope()
            if handler.name:
                slot = self.bind_or_assign(handler.name)
                self.emit({"op": "StoreLocal", "local": slot}, handler)
            else:
                self.emit({"op": "Pop"}, handler)
            if handler.type is not None and not is_exception_type(handler.type):
                raise CompileError("except must catch `Exception`")
            self.block(handler.body)
            self.pop_scope()
        else:
            ex = self.alloc()
            self.emit({"op": "StoreLocal", "local": ex}, statement)
            self.block(statement.finalbody)
            self.emit({"op": "LoadLocal", "local": ex}, statement)
            self.emit({"op": "Throw"}, statement)
        self.patch(jump_join, self.pc())
        if statement.handlers and statement.finalbody:
            self.block(statement.finalbody)

    def raise_statement(self, statement: ast.Raise) -> None:
        if statement.cause is not None:
            raise CompileError("`raise from` is not supported")
        if statement.exc is None:
            raise CompileError("bare `raise` is not supported")
        if (
            isinstance(statement.exc, ast.Call)
            and isinstance(statement.exc.func, ast.Name)
            and statement.exc.func.id == "Exception"
            and len(statement.exc.args) == 1
        ):
            self.expression(statement.exc.args[0])
            self.emit({"op": "Throw"}, statement)
            return
        if isinstance(statement.exc, ast.Name):
            self.expression(statement.exc)
            self.emit({"op": "Throw"}, statement)
            return
        raise CompileError("`raise` must use `Exception(...)` or a local")

    def block(self, body: list[ast.stmt]) -> None:
        self.push_scope()
        for statement in body:
            self.statement(statement)
        self.pop_scope()

    def condition(self, node: ast.expr) -> None:
        refuse_collection_truthiness(node)
        self.expression(node)

    def expression(self, node: ast.expr) -> None:
        node = unwrap(node)
        if isinstance(node, ast.Name):
            if node.id == "None":
                self.emit({"op": "LoadConst", "value": {"t": "null"}}, node)
                return
            if node.id == "True":
                self.emit({"op": "LoadConst", "value": {"t": "bool", "v": True}}, node)
                return
            if node.id == "False":
                self.emit({"op": "LoadConst", "value": {"t": "bool", "v": False}}, node)
                return
            self.emit({"op": "LoadLocal", "local": self.lookup(node.id)}, node)
            return
        literal = const_value(node)
        if literal is not None:
            self.emit({"op": "LoadConst", "value": literal}, node)
            return
        if isinstance(node, ast.Await):
            self.durable(node)
            return
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not):
            refuse_collection_truthiness(node.operand)
            self.expression(node.operand)
            self.emit({"op": "Not"}, node)
            return
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub) and isinstance(
            node.operand, ast.Constant
        ) and isinstance(node.operand.value, (int, float)):
            number = as_number(-node.operand.value)
            self.emit({"op": "LoadConst", "value": {"t": "number", "v": number}}, node)
            return
        if isinstance(node, ast.Compare):
            self.compare(node)
            return
        if isinstance(node, ast.Dict):
            self.dict_literal(node)
            return
        if isinstance(node, ast.List):
            self.emit({"op": "NewArray"}, node)
            for element in node.elts:
                if isinstance(element, ast.Starred):
                    raise CompileError("spread is not supported")
                self.expression(element)
                self.emit({"op": "ArrayPush"}, element)
            return
        if isinstance(node, ast.Attribute):
            self.expression(node.value)
            self.emit({"op": "GetProp", "key": node.attr}, node)
            return
        if isinstance(node, ast.Subscript):
            self.expression(node.value)
            key = string_literal(node.slice, "property name")
            self.emit({"op": "GetProp", "key": key}, node)
            return
        if isinstance(node, ast.BoolOp):
            raise CompileError("logical `and`/`or` are not supported; use nested `if`")
        if isinstance(node, ast.BinOp):
            raise CompileError("arithmetic is not supported")
        if isinstance(node, ast.Lambda):
            raise CompileError("lambdas are not compiled; pass them only as effect callbacks")
        if isinstance(node, ast.Call):
            raise CompileError("call is not a resolved `trigora` durable operation")
        raise CompileError(f"unsupported expression: {type(node).__name__}")

    def compare(self, node: ast.Compare) -> None:
        if len(node.ops) != 1 or len(node.comparators) != 1:
            raise CompileError("chained comparisons are not supported")
        op = node.ops[0]
        right = node.comparators[0]
        if isinstance(op, ast.Is) or isinstance(op, ast.IsNot):
            if not is_none_constant(right) and not is_none_constant(node.left):
                raise CompileError("`is` is only supported with `None`")
            self.expression(node.left)
            self.expression(right)
            self.emit({"op": "StrictNeq" if isinstance(op, ast.IsNot) else "StrictEq"}, node)
            return
        mapped = {
            ast.Eq: "StrictEq",
            ast.NotEq: "StrictNeq",
            ast.Lt: "Lt",
            ast.LtE: "Le",
            ast.Gt: "Gt",
            ast.GtE: "Ge",
        }.get(type(op))
        if mapped is None:
            raise CompileError(f"unsupported operator: {type(op).__name__}")
        self.expression(node.left)
        self.expression(right)
        self.emit({"op": mapped}, node)

    def dict_literal(self, node: ast.Dict) -> None:
        self.emit({"op": "NewObject"}, node)
        for key_node, value_node in zip(node.keys, node.values):
            if key_node is None:
                raise CompileError("dict spread is not supported")
            key = string_literal(key_node, "object key")
            self.expression(value_node)
            self.emit({"op": "SetProp", "key": key}, node)

    def durable(self, node: ast.Await) -> None:
        if not isinstance(node.value, ast.Call):
            raise CompileError("await a `trigora` call")
        call = node.value
        durable = self.resolve_durable(call.func)
        if durable == "effect":
            if len(call.args) != 2:
                raise CompileError("`effect` takes a key and a callback")
            key = string_literal(call.args[0], "effect key")
            self.emit({"op": "LoadConst", "value": {"t": "string", "v": key}}, call)
            self.emit({"op": "Effect"}, call)
            return
        if durable == "wait_for_event":
            if len(call.args) != 1:
                raise CompileError("`wait_for_event` takes an event name")
            name = string_literal(call.args[0], "event name")
            self.emit({"op": "LoadConst", "value": {"t": "string", "v": name}}, call)
            self.emit({"op": "WaitForEvent"}, call)
            return
        if durable == "sleep":
            if len(call.args) != 1:
                raise CompileError("`sleep` takes a duration in milliseconds")
            self.expression(call.args[0])
            self.emit({"op": "Sleep"}, call)
            return
        if durable == "invoke":
            if len(call.args) != 1:
                raise CompileError("`invoke` takes a flow name")
            name = string_literal(call.args[0], "invoke name")
            self.emit({"op": "LoadConst", "value": {"t": "string", "v": name}}, call)
            self.emit({"op": "Invoke"}, call)
            return
        raise CompileError("call is not a resolved `trigora` durable operation")

    def resolve_durable(self, node: ast.expr) -> str | None:
        if isinstance(node, ast.Name):
            return self.aliases.get(node.id)
        if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name):
            raise CompileError("import durable operations from `trigora`")
        return None

    def seal(self) -> None:
        length = len(self.instructions)
        needs_nop = False
        for instruction in self.instructions:
            if instruction.get("target") == length:
                needs_nop = True
            if instruction.get("op") == "PushTry" and (
                instruction.get("catch") == length or instruction.get("finally") == length
            ):
                needs_nop = True
        if needs_nop:
            self.instructions.append({"op": "Nop"})
            self.spans.append(None)


def unwrap(node: ast.expr) -> ast.expr:
    while isinstance(node, ast.UnaryOp) is False and hasattr(node, "value") is False:
        break
    return node


def const_value(node: ast.expr) -> dict[str, Any] | None:
    if not isinstance(node, ast.Constant):
        return None
    value = node.value
    if value is None:
        return {"t": "null"}
    if isinstance(value, bool):
        return {"t": "bool", "v": value}
    if isinstance(value, (int, float)):
        return {"t": "number", "v": as_number(value)}
    if isinstance(value, str):
        return {"t": "string", "v": value}
    raise CompileError("unsupported constant")


def as_number(value: int | float) -> float:
    number = float(value)
    if not math.isfinite(number):
        raise CompileError("non-finite numbers are not supported")
    if isinstance(value, int) and int(number) != value:
        raise CompileError("integer is not an exact IEEE-754 binary64 value")
    return number


def string_literal(node: ast.expr, label: str) -> str:
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    raise CompileError(f"{label} must be a string literal")


def is_none_constant(node: ast.expr) -> bool:
    return isinstance(node, ast.Constant) and node.value is None


def is_exception_type(node: ast.expr) -> bool:
    return isinstance(node, ast.Name) and node.id == "Exception"


def refuse_collection_truthiness(node: ast.expr) -> None:
    node = unwrap(node)
    if isinstance(node, (ast.List, ast.Dict, ast.Tuple, ast.Set)):
        raise CompileError("collection truthiness is not supported; compare explicitly")


def required_from(instructions: list[dict[str, Any]]) -> tuple[list[str], list[str]]:
    engine = ["ts.control_flow"]
    host = ["host.persist_checkpoint"]

    def add_engine(feature: str) -> None:
        if feature not in engine:
            engine.append(feature)

    def add_host(capability: str) -> None:
        if capability not in host:
            host.append(capability)

    for instruction in instructions:
        op = instruction["op"]
        if op == "Effect":
            add_engine("durable.effect")
            add_host("host.effect")
        elif op == "WaitForEvent":
            add_engine("durable.wait_for_event")
            add_host("host.event")
        elif op == "Sleep":
            add_engine("durable.sleep")
            add_host("host.timer")
        elif op == "Invoke":
            add_engine("durable.invoke")
            add_host("host.child")
        elif op in {"Throw", "PushTry", "PopTry"}:
            add_engine("ts.exceptions")
    return engine, host


def span_of(filename: str, node: ast.AST) -> dict[str, Any]:
    return {
        "file": filename,
        "start_line": getattr(node, "lineno", 1),
        "start_column": getattr(node, "col_offset", 0) + 1,
        "end_line": getattr(node, "end_lineno", getattr(node, "lineno", 1)),
        "end_column": getattr(node, "end_col_offset", 0) + 1,
    }
