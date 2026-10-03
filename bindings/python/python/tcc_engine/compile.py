# Copyright (c) 2026 Trigora, Inc.
# SPDX-License-Identifier: BUSL-1.1
# See LICENSE for full terms.

"""Compile a supported Python subset into a TCC artifact."""

from __future__ import annotations

import ast
import hashlib
import math
from typing import Any

from .canonical import canonical_stringify

PACKAGE_VERSION = "26.10.1"
ENGINE_FORMAT_VERSION = 1
HOST_PROTOCOL_VERSION = 1
FRONTEND_IDENTITY = "python"
FRONTEND_ID = FRONTEND_IDENTITY
FRONTEND_VERSION = PACKAGE_VERSION
LANGUAGE_SEMANTICS_VERSION = "py.subset.v1"
SDK_MODULE = "trigora"
PRIMITIVES_MODULE = "tcc_engine.primitives"
DURABLE_MODULES = {SDK_MODULE, PRIMITIVES_MODULE}
DURABLE = {"program", "effect", "wait_for_event", "sleep", "invoke", "gather", "race"}
PROGRAM_ERROR = "A TCC program must declare exactly one `@program` async function."


class CompileError(Exception):
    def __init__(
        self,
        message: str,
        *,
        filename: str = "input.py",
        span: dict[str, Any] | None = None,
        why: str | None = None,
        alternative: str | None = None,
        node: ast.AST | None = None,
    ) -> None:
        super().__init__(message)
        self.filename = filename
        self.span = span if span is not None else (span_of(filename, node) if node is not None else None)
        self.why = why
        self.alternative = alternative
        self.frontend_id = FRONTEND_ID
        self.frontend_version = FRONTEND_VERSION
        self.language_semantics_version = LANGUAGE_SEMANTICS_VERSION


WHY_SUBSET = "it cannot cross a durable checkpoint in the current Python subset"


def compile(source: str, filename: str = "input.py") -> dict[str, Any]:
    try:
        tree = ast.parse(source, filename=filename)
    except SyntaxError as err:
        raise CompileError(f"{filename}: {err.msg}", filename=filename, span=_syntax_span(filename, err)) from err
    aliases = collect_imports(tree, filename)
    entry, helpers = collect_functions(tree, filename, aliases)
    recs = analyze_functions(entry, helpers, aliases, filename)
    functions: dict[str, tuple[int, int, int, bool]] = {}
    for helper in helpers:
        rec = recs[id(helper)]
        count, required = param_shape(helper, filename)
        functions[helper.name] = (rec["id"], count, required, rec["closure"])
    entry_id = recs[id(entry)]["id"]
    lowered = [
        lower_function(rec["node"], aliases, filename, rec, rec["node"] is entry, functions, recs)
        for rec in sorted(recs.values(), key=lambda item: item["id"])
    ]
    instructions = [item for function in lowered for item in function["instructions"]]
    engine, host = required_from(instructions)
    if any(function.get("param_defaults") for function in lowered):
        if "lang.compute" not in engine:
            engine.append("lang.compute")
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
        "program": {"entry": entry_id, "functions": lowered},
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


def _syntax_span(filename: str, err: SyntaxError) -> dict[str, Any] | None:
    if err.lineno is None:
        return None
    return {
        "file": filename,
        "start_line": err.lineno,
        "start_column": err.offset or 1,
        "end_line": getattr(err, "end_lineno", None) or err.lineno,
        "end_column": getattr(err, "end_offset", None) or (err.offset or 1),
    }


def collect_imports(tree: ast.Module, filename: str) -> dict[str, str]:
    aliases: dict[str, str] = {}
    for statement in tree.body:
        if isinstance(statement, ast.ImportFrom):
            if statement.module not in DURABLE_MODULES or statement.level:
                raise CompileError(
                    f"unsupported import from `{statement.module or '?'}`",
                    filename=filename,
                    why=WHY_SUBSET,
                    node=statement,
                )
            for alias in statement.names:
                if alias.name not in DURABLE:
                    raise CompileError(
                        f"unknown durable import `{alias.name}`",
                        filename=filename,
                        why=WHY_SUBSET,
                        node=statement,
                    )
                aliases[alias.asname or alias.name] = alias.name
            continue
        if isinstance(statement, ast.Import):
            raise CompileError("unsupported import", filename=filename, why=WHY_SUBSET, node=statement)
    return aliases


def param_shape(entry: ast.FunctionDef | ast.AsyncFunctionDef, filename: str) -> tuple[int, int]:
    args = entry.args
    if args.posonlyargs or args.kwonlyargs or args.vararg or args.kwarg:
        raise CompileError(
            "parameters must be plain identifiers",
            filename=filename,
            why=WHY_SUBSET,
            node=entry,
        )
    return len(args.args), len(args.args) - len(args.defaults)


def param_default_values(
    entry: ast.FunctionDef | ast.AsyncFunctionDef, filename: str
) -> list[dict[str, Any] | None]:
    count, _required = param_shape(entry, filename)
    defaults: list[dict[str, Any] | None] = [None] * count
    offset = count - len(entry.args.defaults)
    for index, default in enumerate(entry.args.defaults):
        value = const_value(default)
        if (
            value is None
            or value["t"] not in {"null", "bool", "number", "string"}
            or (value["t"] == "number" and not math.isfinite(value["v"]))
        ):
            raise CompileError(
                "v0.1.0 Python defaults are compile-time constants: None, bool, finite number, or string",
                filename=filename,
                why=WHY_SUBSET,
                alternative="compute the default in the function body",
                node=default,
            )
        defaults[offset + index] = value
    return defaults


def analyze_functions(
    entry: ast.AsyncFunctionDef,
    helpers: list[ast.FunctionDef],
    aliases: dict[str, str],
    filename: str,
) -> dict[int, dict[str, Any]]:
    recs: dict[int, dict[str, Any]] = {}
    helper_names = {helper.name for helper in helpers}
    next_id = 1 + len(helpers)

    def make(node: ast.AST, function_id: int, name: str, closure: bool) -> dict[str, Any]:
        rec = {
            "id": function_id,
            "name": name,
            "node": node,
            "parent": None,
            "closure": closure,
            "assigned": set(),
            "nonlocals": set(),
            "captured": [],
            "captured_locals": set(),
        }
        recs[id(node)] = rec
        return rec

    make(entry, 0, entry.name, False)
    for index, helper in enumerate(helpers):
        make(helper, index + 1, helper.name, False)

    def scan(node: ast.AST) -> tuple[set[str], set[str], list[ast.AST]]:
        assigned: set[str] = set()
        nonlocals: set[str] = set()
        nested: list[ast.AST] = []
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)):
            assigned.update(arg.arg for arg in node.args.args)

        def visit_stmt(statement: ast.stmt) -> None:
            if isinstance(statement, ast.Global):
                raise CompileError("`global` is not supported", filename=filename, why=WHY_SUBSET, node=statement)
            if isinstance(statement, ast.Nonlocal):
                nonlocals.update(statement.names)
                return
            if isinstance(statement, ast.FunctionDef):
                assigned.add(statement.name)
                nested.append(statement)
                return
            if isinstance(statement, ast.AsyncFunctionDef):
                raise CompileError(
                    "nested functions cannot be async",
                    filename=filename,
                    why=WHY_SUBSET,
                    alternative="keep durable operations in the program entry",
                    node=statement,
                )
            if isinstance(statement, ast.Assign):
                for target in statement.targets:
                    collect_target(target)
            elif isinstance(statement, ast.AugAssign):
                collect_target(statement.target)
            elif isinstance(statement, ast.For):
                collect_target(statement.target)
            for child in ast.iter_child_nodes(statement):
                if isinstance(child, ast.stmt):
                    visit_stmt(child)
                elif isinstance(child, ast.expr):
                    visit_expr(child)

        def visit_expr(expr: ast.expr) -> None:
            if isinstance(expr, ast.Call) and effect_callback(expr) is not None:
                for argument in expr.args[:1]:
                    visit_expr(argument)
                return
            if isinstance(expr, ast.Lambda):
                nested.append(expr)
                return
            for child in ast.iter_child_nodes(expr):
                if isinstance(child, ast.expr):
                    visit_expr(child)
                elif isinstance(child, ast.stmt):
                    visit_stmt(child)

        def collect_target(target: ast.expr) -> None:
            if isinstance(target, ast.Name):
                assigned.add(target.id)
            elif isinstance(target, (ast.Tuple, ast.List)):
                for element in target.elts:
                    collect_target(element)

        body = node.body if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) else [node.body]
        if isinstance(node, ast.Lambda):
            visit_expr(node.body)
        else:
            for statement in body:
                visit_stmt(statement)
        assigned.difference_update(nonlocals)
        return assigned, nonlocals, nested

    def effect_callback(call: ast.Call) -> ast.Lambda | None:
        if len(call.args) < 2 or not isinstance(call.args[1], ast.Lambda):
            return None
        func = call.func
        if isinstance(func, ast.Name) and aliases.get(func.id, func.id) == "effect":
            return call.args[1]
        return None

    def loads(node: ast.AST) -> list[tuple[str, ast.AST]]:
        found: list[tuple[str, ast.AST]] = []

        def visit_stmt(statement: ast.stmt) -> None:
            if isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Global, ast.Nonlocal)):
                return
            for child in ast.iter_child_nodes(statement):
                if isinstance(child, ast.stmt):
                    visit_stmt(child)
                elif isinstance(child, ast.expr):
                    visit_expr(child)

        def visit_expr(expr: ast.expr) -> None:
            if isinstance(expr, ast.Call) and effect_callback(expr) is not None:
                for argument in expr.args[:1]:
                    visit_expr(argument)
                return
            if isinstance(expr, ast.Lambda):
                return
            if isinstance(expr, ast.Name) and isinstance(expr.ctx, ast.Load):
                found.append((expr.id, expr))
            for child in ast.iter_child_nodes(expr):
                if isinstance(child, ast.expr):
                    visit_expr(child)

        if isinstance(node, ast.Lambda):
            visit_expr(node.body)
        else:
            for statement in node.body:
                visit_stmt(statement)
        return found

    def note(fn: dict[str, Any], name: str, node: ast.AST) -> None:
        owner = fn["parent"]
        while owner is not None and name not in owner["assigned"]:
            owner = owner["parent"]
        if owner is None:
            if name in helper_names and not isinstance(getattr(node, "parent_call", None), ast.Call):
                pass
            if name in helper_names:
                parent = getattr(node, "_tcc_call", None)
                if parent is None:
                    for helper in helpers:
                        if helper.name == name:
                            recs[id(helper)]["closure"] = True
                return
            return
        owner["captured_locals"].add(name)
        mid = fn
        while mid is not owner:
            if name not in mid["captured"]:
                mid["captured"].append(name)
            mid = mid["parent"]

    def walk(rec: dict[str, Any]) -> None:
        assigned, nonlocals, nested = scan(rec["node"])
        if nonlocals & set(arg.arg for arg in rec["node"].args.args):
            raise CompileError("a parameter cannot be nonlocal", filename=filename, why=WHY_SUBSET, node=rec["node"])
        rec["assigned"] = assigned
        rec["nonlocals"] = nonlocals
        nonlocal next_id
        for child in nested:
            if isinstance(child, ast.Lambda):
                child_rec = make(child, next_id, f"lambda{next_id}", True)
            else:
                child_rec = make(child, next_id, child.name, True)
            next_id += 1
            child_rec["parent"] = rec
            walk(child_rec)
        for name in nonlocals:
            owner = rec["parent"]
            while owner is not None and name not in owner["assigned"]:
                owner = owner["parent"]
            if owner is None:
                raise CompileError(
                    f"nonlocal `{name}` is not an enclosing binding",
                    filename=filename,
                    why=WHY_SUBSET,
                    node=rec["node"],
                )
            note(rec, name, rec["node"])
        for name, expr in loads(rec["node"]):
            if name in {"None", "True", "False"} or name in rec["assigned"]:
                continue
            if name in nonlocals:
                continue
            parent = expr.parent if hasattr(expr, "parent") else None
            called = isinstance(parent, ast.Call) and parent.func is expr
            if name in helper_names and rec["parent"] is None:
                if not called:
                    recs[id(next(helper for helper in helpers if helper.name == name))]["closure"] = True
                continue
            if name in helper_names and not any(
                ancestor is not None and name in ancestor["assigned"]
                for ancestor in _ancestors(rec)
            ):
                if not called:
                    recs[id(next(helper for helper in helpers if helper.name == name))]["closure"] = True
                if called:
                    continue
                continue
            note(rec, name, expr)

    for rec in list(recs.values()):
        if rec["parent"] is None:
            walk(rec)
    return recs


def _ancestors(rec: dict[str, Any]) -> list[dict[str, Any] | None]:
    found = []
    current = rec["parent"]
    while current is not None:
        found.append(current)
        current = current["parent"]
    return found


def _marks_program(decorator: ast.expr, aliases: dict[str, str]) -> bool:
    return isinstance(decorator, ast.Name) and aliases.get(decorator.id) == "program"


def collect_functions(
    tree: ast.Module, filename: str, aliases: dict[str, str]
) -> tuple[ast.AsyncFunctionDef, list[ast.FunctionDef]]:
    entries: list[ast.AsyncFunctionDef] = []
    helpers: list[ast.FunctionDef] = []
    for statement in tree.body:
        if isinstance(statement, ast.ImportFrom):
            continue
        if isinstance(statement, ast.AsyncFunctionDef):
            marked = [item for item in statement.decorator_list if _marks_program(item, aliases)]
            if len(statement.decorator_list) == 1 and marked:
                param_shape(statement, filename)
                entries.append(statement)
                continue
            raise CompileError(PROGRAM_ERROR, filename=filename, why=WHY_SUBSET, node=statement)
        if isinstance(statement, ast.FunctionDef):
            if any(_marks_program(item, aliases) for item in statement.decorator_list):
                raise CompileError(PROGRAM_ERROR, filename=filename, why=WHY_SUBSET, node=statement)
            if statement.decorator_list:
                raise CompileError("decorators are not supported", filename=filename, why=WHY_SUBSET, node=statement)
            param_shape(statement, filename)
            helpers.append(statement)
            continue
        raise CompileError(
            f"unsupported top-level statement: {type(statement).__name__}",
            filename=filename,
            why=WHY_SUBSET,
            node=statement,
        )
    if len(entries) != 1:
        raise CompileError(PROGRAM_ERROR, filename=filename, why=WHY_SUBSET)
    return entries[0], helpers


def lower_function(
    entry: ast.FunctionDef | ast.AsyncFunctionDef | ast.Lambda,
    aliases: dict[str, str],
    filename: str,
    rec: dict[str, Any],
    allow_durable: bool,
    functions: dict[str, tuple[int, int, int, bool]],
    recs: dict[int, dict[str, Any]],
) -> dict[str, Any]:
    lower = Lowerer(aliases, filename, allow_durable, functions, rec, recs)
    lower.push_scope()
    if rec["closure"]:
        lower.alloc()
    for index, name in enumerate(rec["captured"]):
        lower.outers[name] = index
    if isinstance(entry, ast.Lambda):
        if entry.args.defaults or entry.args.vararg or entry.args.kwarg or entry.args.kwonlyargs:
            raise CompileError("lambda parameters must be plain identifiers", filename=filename, why=WHY_SUBSET, node=entry)
        defaults: list[dict[str, Any] | None] = []
        for parameter in entry.args.args:
            lower.declare(parameter.arg)
        for parameter in entry.args.args:
            if parameter.arg in rec["captured_locals"]:
                lower.box_param(parameter.arg, entry)
        lower.prepare_cells(entry)
        lower.expression(entry.body)
        lower.emit({"op": "Return"}, entry)
        name = rec["name"]
        param_count = len(entry.args.args)
    else:
        defaults = param_default_values(entry, filename)
        for parameter in entry.args.args:
            lower.declare(parameter.arg)
        for parameter in entry.args.args:
            if parameter.arg in rec["captured_locals"]:
                lower.box_param(parameter.arg, entry)
        lower.prepare_cells(entry)
        for statement in entry.body:
            lower.statement(statement)
        if lower.needs_implicit_return():
            lower.emit({"op": "LoadConst", "value": {"t": "null"}}, entry)
            lower.emit({"op": "Return"}, entry)
        name = entry.name
        param_count = len(entry.args.args)
    lower.pop_scope()
    lower.seal()
    function: dict[str, Any] = {
        "id": rec["id"],
        "name": name,
        "param_count": param_count,
        "local_count": lower.max_slots,
        "instructions": lower.instructions,
        "spans": lower.spans,
    }
    if any(item is not None for item in defaults):
        function["param_defaults"] = defaults
    return function


class Loop:
    def __init__(self, unwatch: bool = False) -> None:
        self.breaks: list[int] = []
        self.continues: list[int] = []
        self.continue_target: int | None = None
        self.unwatch = unwatch


class Lowerer:
    def __init__(
        self,
        aliases: dict[str, str],
        filename: str,
        allow_durable: bool = True,
        functions: dict[str, tuple[int, int, int, bool]] | None = None,
        rec: dict[str, Any] | None = None,
        recs: dict[int, dict[str, Any]] | None = None,
    ) -> None:
        self.aliases = aliases
        self.filename = filename
        self.allow_durable = allow_durable
        self.functions = functions or {}
        self.rec = rec or {
            "id": 0,
            "captured": [],
            "captured_locals": set(),
            "closure": False,
            "assigned": set(),
            "nonlocals": set(),
        }
        self.recs = recs or {}
        self.instructions: list[dict[str, Any]] = []
        self.spans: list[dict[str, Any] | None] = []
        self.scopes: list[dict[str, int]] = []
        self.slots: dict[str, int] = {}
        self.outers: dict[str, int] = {}
        self.cells: set[str] = set()
        self.loops: list[Loop] = []
        self.max_slots = 0

    def fail(
        self,
        node: ast.AST,
        message: str,
        why: str | None = None,
        alternative: str | None = None,
    ) -> None:
        raise CompileError(
            message,
            filename=self.filename,
            why=why,
            alternative=alternative,
            node=node,
        )

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

    def load_name(self, name: str, node: ast.AST) -> None:
        if name in self.outers:
            self.emit({"op": "LoadLocal", "local": 0}, node)
            self.emit({"op": "EnvGet", "index": self.outers[name]}, node)
            return
        if name in self.cells:
            self.emit({"op": "LoadLocal", "local": self.lookup(name)}, node)
            self.emit({"op": "EnvGet", "index": 0}, node)
            return
        if name in self.slots:
            self.emit({"op": "LoadLocal", "local": self.slots[name]}, node)
            return
        info = self.functions.get(name)
        if info is not None and info[3]:
            self.emit({"op": "LoadFunc", "func": info[0]}, node)
            return
        raise CompileError(f"unknown local `{name}`")

    def store_name(self, name: str, node: ast.AST) -> None:
        if name in self.outers or name in self.rec["nonlocals"]:
            index = self.outers.get(name)
            if index is None:
                raise CompileError(f"nonlocal `{name}` is not an enclosing binding")
            temp = self.alloc()
            self.emit({"op": "StoreLocal", "local": temp}, node)
            self.emit({"op": "LoadLocal", "local": 0}, node)
            self.emit({"op": "LoadLocal", "local": temp}, node)
            self.emit({"op": "EnvSet", "index": index}, node)
            return
        slot = self.bind_or_assign(name)
        if name in self.rec["captured_locals"]:
            temp = self.alloc()
            self.emit({"op": "StoreLocal", "local": temp}, node)
            self.emit({"op": "LoadLocal", "local": slot}, node)
            self.emit({"op": "LoadLocal", "local": temp}, node)
            self.emit({"op": "EnvSet", "index": 0}, node)
            return
        self.emit({"op": "StoreLocal", "local": slot}, node)

    def prepare_cells(self, node: ast.AST) -> None:
        for name in self.rec["captured_locals"]:
            if name in self.cells or name in self.outers:
                continue
            slot = self.bind_or_assign(name)
            self.emit({"op": "LoadConst", "value": {"t": "null"}}, node)
            self.emit({"op": "NewCell"}, node)
            self.emit({"op": "NewEnv", "count": 1}, node)
            self.emit({"op": "StoreLocal", "local": slot}, node)
            self.cells.add(name)

    def box_param(self, name: str, node: ast.AST) -> None:
        slot = self.lookup(name)
        self.emit({"op": "LoadLocal", "local": slot}, node)
        self.emit({"op": "NewCell"}, node)
        self.emit({"op": "NewEnv", "count": 1}, node)
        self.emit({"op": "StoreLocal", "local": slot}, node)
        self.cells.add(name)

    def emit_cell_ref(self, name: str, node: ast.AST) -> None:
        if name in self.outers:
            self.emit({"op": "LoadLocal", "local": 0}, node)
            self.emit({"op": "EnvSlot", "index": self.outers[name]}, node)
            return
        self.emit({"op": "LoadLocal", "local": self.lookup(name)}, node)
        self.emit({"op": "EnvSlot", "index": 0}, node)

    def emit_closure(self, rec: dict[str, Any], node: ast.AST) -> None:
        for name in rec["captured"]:
            self.emit_cell_ref(name, node)
        self.emit({"op": "NewEnv", "count": len(rec["captured"])}, node)
        self.emit({"op": "NewClosure", "func": rec["id"]}, node)

    def bind_or_assign(self, name: str) -> int:
        if name in self.slots:
            return self.slots[name]
        return self.declare(name)

    def statement(self, statement: ast.stmt) -> None:
        if isinstance(statement, ast.Pass) or isinstance(statement, ast.Nonlocal):
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
            self.aug_assign(statement)
            return
        if isinstance(statement, ast.Return):
            self.unwatch_all(statement)
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
            loop = self.loops[-1]
            if loop.unwatch:
                self.emit({"op": "UnwatchIter"}, statement)
            loop.breaks.append(self.emit({"op": "Jump", "target": 0}, statement))
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
            self.for_statement(statement)
            return
        if isinstance(statement, ast.FunctionDef) or isinstance(statement, ast.AsyncFunctionDef):
            if isinstance(statement, ast.AsyncFunctionDef):
                self.fail(statement, "nested functions cannot be async", WHY_SUBSET, "keep durable operations in the program entry")
            rec = self.recs.get(id(statement))
            if rec is None:
                self.fail(statement, "nested functions are not supported", WHY_SUBSET)
            self.emit_closure(rec, statement)
            self.store_name(statement.name, statement)
            return
        if isinstance(statement, ast.ClassDef):
            self.fail(statement, "classes are not supported", WHY_SUBSET)
        self.fail(statement, f"unsupported statement: {type(statement).__name__}", WHY_SUBSET)

    def assign(self, statement: ast.Assign) -> None:
        if len(statement.targets) != 1:
            raise CompileError("assignment target must be a local, property, index, or one-level unpack")
        target = statement.targets[0]
        if isinstance(target, ast.Name):
            self.expression(statement.value)
            self.store_name(target.id, statement)
            return
        if isinstance(target, ast.Tuple):
            self.unpack(target, statement.value, statement)
            return
        self.store_target(target, statement.value, statement, compound=None)

    def aug_assign(self, statement: ast.AugAssign) -> None:
        op = {
            ast.Add: "Add",
            ast.Sub: "Sub",
            ast.Mult: "Mul",
            ast.Div: "Div",
            ast.FloorDiv: "FloorDiv",
            ast.Mod: "Rem",
            ast.Pow: "Pow",
        }.get(type(statement.op))
        if op is None:
            raise CompileError("unsupported augmented assignment")
        self.store_target(statement.target, statement.value, statement, compound=op)

    def unpack(self, target: ast.Tuple, value: ast.expr, node: ast.AST) -> None:
        names: list[str] = []
        for element in target.elts:
            if not isinstance(element, ast.Name):
                raise CompileError("unpacking bindings must be simple names")
            names.append(element.id)
        for name in names:
            if name not in self.outers:
                self.bind_or_assign(name)
        self.expression(value)
        for index, name in enumerate(names):
            self.emit({"op": "ArrayIndex", "index": index}, node)
            self.store_name(name, node)
        self.emit({"op": "Pop"}, node)

    def store_target(self, target: ast.expr, value: ast.expr, node: ast.AST, compound: str | None) -> None:
        if isinstance(target, ast.Name):
            if compound:
                self.load_name(target.id, target)
                self.expression(value)
                self.emit({"op": compound}, node)
            else:
                self.expression(value)
            self.store_name(target.id, node)
            return
        if isinstance(target, ast.Attribute):
            base = self.alloc()
            self.expression(target.value)
            self.emit({"op": "StoreLocal", "local": base}, target)
            if compound:
                self.emit({"op": "LoadLocal", "local": base}, target)
                self.emit({"op": "GetProp", "key": target.attr}, target)
                self.expression(value)
                self.emit({"op": compound}, node)
            else:
                self.expression(value)
            stored = self.alloc()
            self.emit({"op": "StoreLocal", "local": stored}, node)
            self.emit({"op": "LoadLocal", "local": base}, target)
            self.emit({"op": "LoadLocal", "local": stored}, node)
            self.emit({"op": "SetProp", "key": target.attr}, target)
            self.emit({"op": "Pop"}, node)
            return
        if isinstance(target, ast.Subscript):
            base = self.alloc()
            self.expression(target.value)
            self.emit({"op": "StoreLocal", "local": base}, target)
            key = const_value(target.slice)
            if key is not None and key["t"] == "string":
                if compound:
                    self.emit({"op": "LoadLocal", "local": base}, target)
                    self.emit({"op": "GetProp", "key": key["v"]}, target)
                    self.expression(value)
                    self.emit({"op": compound}, node)
                else:
                    self.expression(value)
                stored = self.alloc()
                self.emit({"op": "StoreLocal", "local": stored}, node)
                self.emit({"op": "LoadLocal", "local": base}, target)
                self.emit({"op": "LoadLocal", "local": stored}, node)
                self.emit({"op": "SetProp", "key": key["v"]}, target)
                self.emit({"op": "Pop"}, node)
                return
            index = self.alloc()
            self.expression(target.slice)
            self.emit({"op": "StoreLocal", "local": index}, target)
            if compound:
                self.emit({"op": "LoadLocal", "local": base}, target)
                self.emit({"op": "LoadLocal", "local": index}, target)
                self.emit({"op": "GetIndex"}, target)
                self.expression(value)
                self.emit({"op": compound}, node)
            else:
                self.expression(value)
            stored = self.alloc()
            self.emit({"op": "StoreLocal", "local": stored}, node)
            self.emit({"op": "LoadLocal", "local": base}, target)
            self.emit({"op": "LoadLocal", "local": index}, target)
            self.emit({"op": "LoadLocal", "local": stored}, node)
            self.emit({"op": "SetIndex"}, target)
            self.emit({"op": "Pop"}, node)
            return
        raise CompileError("assignment target must be a local, property, index, or one-level unpack")

    def unwatch_all(self, node: ast.AST) -> None:
        for loop in reversed(self.loops):
            if loop.unwatch:
                self.emit({"op": "UnwatchIter"}, node)

    def for_statement(self, statement: ast.For) -> None:
        if statement.orelse:
            raise CompileError("`for/else` is not supported")
        if not isinstance(statement.target, ast.Name):
            raise CompileError("`for` binding must be a name")
        self.bind_or_assign(statement.target.id)
        index = self.alloc()
        length = self.alloc()
        self.expression(statement.iter)
        self.emit({"op": "WatchIter"}, statement.iter)
        self.emit({"op": "Length"}, statement.iter)
        self.emit({"op": "StoreLocal", "local": length}, statement)
        self.emit({"op": "LoadConst", "value": {"t": "number", "v": 0}}, statement)
        self.emit({"op": "StoreLocal", "local": index}, statement)
        loop = Loop(unwatch=True)
        self.loops.append(loop)
        cond = self.pc()
        loop.continue_target = None
        self.emit({"op": "LoadLocal", "local": index}, statement)
        self.emit({"op": "LoadLocal", "local": length}, statement)
        self.emit({"op": "Lt"}, statement)
        jump_end = self.emit({"op": "JumpIfFalse", "target": 0}, statement)
        self.expression(statement.iter)
        self.emit({"op": "LoadLocal", "local": index}, statement)
        self.emit({"op": "GetIndex"}, statement)
        self.store_name(statement.target.id, statement.target)
        self.block(statement.body)
        incr = self.pc()
        loop.continue_target = incr
        for site in loop.continues:
            self.patch(site, incr)
        self.emit({"op": "LoadLocal", "local": index}, statement)
        self.emit({"op": "LoadConst", "value": {"t": "number", "v": 1}}, statement)
        self.emit({"op": "Add"}, statement)
        self.emit({"op": "StoreLocal", "local": index}, statement)
        self.emit({"op": "Jump", "target": cond}, statement)
        self.patch(jump_end, self.pc())
        self.emit({"op": "UnwatchIter"}, statement)
        end = self.pc()
        for site in loop.breaks:
            self.patch(site, end)
        self.loops.pop()

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
        refuse_collection_truthiness(node, self.filename)
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
            self.load_name(node.id, node)
            return
        literal = const_value(node)
        if literal is not None:
            self.emit({"op": "LoadConst", "value": literal}, node)
            return
        if isinstance(node, ast.Await):
            self.durable(node)
            return
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not):
            refuse_collection_truthiness(node.operand, self.filename)
            self.expression(node.operand)
            self.emit({"op": "Not"}, node)
            return
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
            if isinstance(node.operand, ast.Constant) and isinstance(node.operand.value, (int, float)):
                number = as_number(-node.operand.value)
                self.emit({"op": "LoadConst", "value": {"t": "number", "v": number}}, node)
                return
            self.expression(node.operand)
            self.emit({"op": "Neg"}, node)
            return
        if isinstance(node, ast.IfExp):
            self.condition(node.test)
            jump_else = self.emit({"op": "JumpIfFalse", "target": 0}, node.test)
            self.expression(node.body)
            jump_end = self.emit({"op": "Jump", "target": 0}, node)
            self.patch(jump_else, self.pc())
            self.expression(node.orelse)
            self.patch(jump_end, self.pc())
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
            self.load_subscript(node)
            return
        if isinstance(node, ast.BoolOp):
            self.bool_op(node)
            return
        if isinstance(node, ast.BinOp):
            self.bin_op(node)
            return
        if isinstance(node, ast.Lambda):
            rec = self.recs.get(id(node))
            if rec is None:
                self.fail(node, "lambdas are not compiled; pass them only as effect callbacks", WHY_SUBSET)
            self.emit_closure(rec, node)
            return
        if isinstance(node, ast.Call):
            if self.lower_call(node):
                return
            name = self.resolve_durable(node.func)
            if name in {"gather", "race"}:
                raise CompileError(f"await `{name}` directly; do not store the call")
            self.call_value(node)
            return
        self.fail(node, f"unsupported expression: {type(node).__name__}", WHY_SUBSET)

    def compare(self, node: ast.Compare) -> None:
        if len(node.ops) != 1 or len(node.comparators) != 1:
            raise CompileError("chained comparisons are not supported")
        op = node.ops[0]
        right = node.comparators[0]
        if isinstance(op, ast.Is) or isinstance(op, ast.IsNot):
            self.expression(node.left)
            self.expression(right)
            self.emit({"op": "Same"}, node)
            if isinstance(op, ast.IsNot):
                self.emit({"op": "Not"}, node)
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

    def load_subscript(self, node: ast.Subscript) -> None:
        self.expression(node.value)
        key = const_value(node.slice)
        if key is not None and key["t"] == "string":
            self.emit({"op": "GetProp", "key": key["v"]}, node)
            return
        self.expression(node.slice)
        self.emit({"op": "GetIndex"}, node)

    def bool_op(self, node: ast.BoolOp) -> None:
        and_op = isinstance(node.op, ast.And)
        self.expression(node.values[0])
        for value in node.values[1:]:
            slot = self.alloc()
            self.emit({"op": "StoreLocal", "local": slot}, node)
            self.emit({"op": "LoadLocal", "local": slot}, node)
            jump = self.emit(
                {"op": "JumpIfFalse" if and_op else "JumpIfTrue", "target": 0},
                node,
            )
            self.expression(value)
            jump_end = self.emit({"op": "Jump", "target": 0}, node)
            self.patch(jump, self.pc())
            self.emit({"op": "LoadLocal", "local": slot}, node)
            self.patch(jump_end, self.pc())

    def bin_op(self, node: ast.BinOp) -> None:
        mapped = {
            ast.Add: "Add",
            ast.Sub: "Sub",
            ast.Mult: "Mul",
            ast.Div: "Div",
            ast.FloorDiv: "FloorDiv",
            ast.Mod: "Rem",
            ast.Pow: "Pow",
        }.get(type(node.op))
        if mapped is None:
            raise CompileError(f"unsupported operator: {type(node.op).__name__}")
        self.expression(node.left)
        self.expression(node.right)
        self.emit({"op": mapped}, node)

    def lower_call(self, node: ast.Call) -> bool:
        if node.keywords or any(isinstance(arg, ast.Starred) for arg in node.args):
            return False
        func = node.func
        if isinstance(func, ast.Name) and func.id == "len" and len(node.args) == 1:
            self.expression(node.args[0])
            self.emit({"op": "Length"}, node)
            return True
        if isinstance(func, ast.Name) and (
            func.id in self.slots or func.id in self.outers or func.id in self.cells
        ):
            return False
        if isinstance(func, ast.Name) and func.id in self.functions:
            function_id, param_count, required, closure = self.functions[func.id]
            argc = len(node.args)
            if argc < required or argc > param_count:
                raise CompileError(f"`{func.id}` expects {required} to {param_count} arguments")
            if closure:
                self.emit({"op": "LoadFunc", "func": function_id}, func)
            for argument in node.args:
                self.expression(argument)
            if closure:
                self.emit({"op": "CallClosure", "argc": argc}, node)
            else:
                self.emit({"op": "Call", "func": function_id, "argc": argc}, node)
            return True
        if (
            isinstance(func, ast.Attribute)
            and func.attr == "append"
            and len(node.args) == 1
        ):
            self.expression(func.value)
            self.expression(node.args[0])
            self.emit({"op": "ArrayPush"}, node)
            return True
        return False

    def call_value(self, node: ast.Call) -> None:
        if node.keywords or any(isinstance(arg, ast.Starred) for arg in node.args):
            raise CompileError("spread and keyword arguments are not supported")
        self.expression(node.func)
        for argument in node.args:
            self.expression(argument)
        self.emit({"op": "CallClosure", "argc": len(node.args)}, node)

    def durable(self, node: ast.Await) -> None:
        if not self.allow_durable:
            self.fail(
                node,
                "v0.1.0 helper calls are non-suspending. Durable boundaries are only allowed in the program entry",
                WHY_SUBSET,
                "move the durable operation into run",
            )
        if not isinstance(node.value, ast.Call):
            raise CompileError("await a durable operation")
        call = node.value
        durable = self.resolve_durable(call.func)
        if durable == "gather":
            self.join_call(call, "JoinAll")
            return
        if durable == "race":
            self.join_call(call, "JoinAny")
            return
        self.emit_durable_call(call)

    def emit_durable_call(self, call: ast.Call) -> None:
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
            if call.keywords or any(isinstance(arg, ast.Starred) for arg in call.args):
                raise CompileError("`invoke` does not take keywords or spread")
            if len(call.args) < 1:
                raise CompileError("`invoke` takes a flow name and optional arguments")
            for argument in call.args[1:]:
                self.expression(argument)
            name = string_literal(call.args[0], "invoke name")
            self.emit({"op": "LoadConst", "value": {"t": "string", "v": name}}, call)
            instruction: dict[str, Any] = {"op": "Invoke"}
            if len(call.args) > 1:
                instruction["arg_count"] = len(call.args) - 1
            self.emit(instruction, call)
            return
        raise CompileError("call is not a resolved durable operation")

    def join_call(self, call: ast.Call, join_op: str) -> None:
        label = "gather" if join_op == "JoinAll" else "race"
        if call.keywords:
            raise CompileError(f"`{label}` does not take keyword arguments")
        if len(call.args) == 0 or len(call.args) > 32:
            raise CompileError(f"`{label}` supports 1 to 32 branches")
        fork = self.emit({"op": "Fork", "count": len(call.args), "join_pc": 0}, call)
        for arg in call.args:
            if isinstance(arg, ast.Starred):
                raise CompileError(f"spread is not supported in `{label}`")
            if isinstance(arg, ast.Await):
                raise CompileError(f"`{label}` arguments must be direct durable calls")
            if not isinstance(arg, ast.Call):
                raise CompileError(f"`{label}` arguments must be direct durable calls")
            branch = self.resolve_durable(arg.func)
            if branch in {"gather", "race"}:
                raise CompileError(f"nested `{branch}` is not supported")
            if branch not in {"effect", "wait_for_event", "sleep", "invoke"}:
                raise CompileError(
                    f"`{label}` arguments must be effect, wait_for_event, sleep, or invoke"
                )
            self.emit_durable_call(arg)
        self.instructions[fork]["join_pc"] = self.pc()
        self.emit({"op": join_op}, call)

    def resolve_durable(self, node: ast.expr) -> str | None:
        if isinstance(node, ast.Name):
            return self.aliases.get(node.id)
        if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name):
            raise CompileError("import durable operations from `tcc_engine.primitives` or `trigora`")
        return None

    def needs_implicit_return(self) -> bool:
        end = self.pc()
        last = self.instructions[-1] if self.instructions else None
        if last is None or last.get("op") != "Return":
            return True
        for instruction in self.instructions:
            if instruction.get("target") == end:
                return True
            if instruction.get("catch") == end or instruction.get("finally") == end:
                return True
            if instruction.get("join_pc") == end:
                return True
        return False

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


def refuse_collection_truthiness(node: ast.expr, filename: str) -> None:
    node = unwrap(node)
    if isinstance(node, (ast.List, ast.Dict, ast.Tuple, ast.Set)):
        raise CompileError(
            "collection truthiness is not supported; compare explicitly",
            filename=filename,
            why=WHY_SUBSET,
            alternative="compare explicitly",
            node=node,
        )


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
        elif op in {"Fork", "JoinAll", "JoinAny"}:
            add_engine("durable.concurrent_group")
        elif op in {"Throw", "PushTry", "PopTry"}:
            add_engine("ts.exceptions")
        elif op in {
            "Add",
            "Sub",
            "Mul",
            "Div",
            "Rem",
            "Neg",
            "Pow",
            "FloorDiv",
            "GetIndex",
            "SetIndex",
            "Length",
            "WatchIter",
            "UnwatchIter",
            "Same",
            "Call",
            "NewCell",
            "NewEnv",
            "NewClosure",
            "EnvGet",
            "EnvSet",
            "EnvSlot",
            "CallClosure",
            "LoadFunc",
        }:
            add_engine("lang.compute")
    return engine, host


def span_of(filename: str, node: ast.AST) -> dict[str, Any]:
    return {
        "file": filename,
        "start_line": getattr(node, "lineno", 1),
        "start_column": getattr(node, "col_offset", 0) + 1,
        "end_line": getattr(node, "end_lineno", getattr(node, "lineno", 1)),
        "end_column": getattr(node, "end_col_offset", 0) + 1,
    }
