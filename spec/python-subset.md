# Python subset

`language_semantics_version` `py.subset.v1`. A construct is supported only when it compiles, runs through the native Python binding, runs through WASM on the same artifact, and recovers from SIGKILL at every durable boundary with the same observable result.

Authoring uses resolved imports from `tcc_engine.primitives` (engine-native) or `trigora` (Trigora product). Both lower to the same durable operations. There is no `ctx` and no `workflow()` wrapper.

## Supported

- Exactly one top-level `@program` async function, any name. `@program` is the identity marker `def program(fn): return fn` imported from `tcc_engine.primitives` or `trigora`, including an alias. Zero, two, a sync `@program`, or a bare `async def` is a compile error. Parameters are plain. Defaults are compile-time constants: `None`, bool, finite number, or string. An omitted argument uses that constant. Explicit `None` stays null. Extra arguments are a start error. A missing argument with no default is a start error. `*args`, keyword-only, positional-only, and mutable literal defaults (`b=[]`, `b={}`) stay unsupported
- Top-level `def` helpers with the same parameter rules. They may call other helpers. They cannot await a durable operation. A durable boundary runs only while the program entry frame is active, so a helper or closure frame is never live at a checkpoint. A top-level helper used as a value is one empty-environment closure for the execution
- Imports from `tcc_engine.primitives` or `trigora`: `program`, `effect`, `wait_for_event`, `sleep`, `invoke`, `gather`, `race` (aliases included). `gather` and `race` are compiler intrinsics for [concurrency.md](concurrency.md), not runtime coroutine helpers.
- Assignment to simple locals
- `if` / `elif` / `else`
- `while`
- Unlabeled `break` / `continue`
- Nested blocks; names map to function-level slots
- `return` of a lowered expression
- `raise Exception(...)`, `try` / `except` / `finally`
- Literals: `None`, booleans, numbers that are exact IEEE-754 binary64 values, strings. This subset has no `float("inf")`
- Dicts with string keys and lists as continuation-heap refs (no spread). Assignment copies the ref. Mutation is in place, including through a helper argument. Cycles are representable in the heap and cannot be inlined across a host boundary
- Property read and write `obj.key` or `obj["key"]`, index read and write `xs[i]` (negative indexes count from the end), `len(xs)`, `xs.append(value)`
- One-level unpacking `a, b = pair` for a list. No rest or nested patterns
- `==` / `!=` (structural on lists and dicts; a cycle in that walk is a runtime error), `is` / `is not` (reference equality, including `is None` and closure identity), unary `not` and unary `-`, numeric `<` `<=` `>` `>=`
- `+` `-` `*` `/` `//` `%` `**` and augmented assignment, including `//=` and `**=`, inside the operator domain below. `and` / `or` short-circuit and return the operand
- `lambda` with one expression and plain parameters, and nested `def`. A name assigned in the nested function is a new local unless it is declared `nonlocal`. A `nonlocal` name that is not an enclosing binding is a compile error. `global` stays unsupported. A read of a name that is not assigned in the nested function captures the outer cell. Loop lambdas share that one loop binding, so they see its final value. A nested function allocates a new closure each time its statement or expression runs. A closure value may live across `await`; its frame may not
- `a if cond else b`
- `for x in xs` over a list. Length is captured once. The index is a real local. Structural mutation of the iterated cell, including through an alias, is a runtime error. Mutating a distinct element is allowed. Not dict iteration, not `for`/`else`
- Sequential and branched mixes of durable operations
- `invoke(name, *args)`. Each argument after the name is a subset value. See [durable-operations.md](durable-operations.md)

`if` / `while` / `not` operands must be bool, number, string, `None`, or a comparison. Collection literals used as conditions are compile errors. Effect keys, event names, and invoke names are string literals. Effect callbacks are not compiled into the artifact.

## Identities

- Effect: `{execution_id}:{key}`. Re-executing the same instruction with the same key skips a completed journal entry.
- Wait: `{execution_id}:{name}:{correlation}:{pc}` (empty correlation is an empty field)
- Child invoke: `{parent}:invoke:{pc}` outside a join. Inside `gather` or `race`, wait, timer, and child ids are the branch id from [concurrency.md](concurrency.md).

`+` `-` `*` `/` `//` `%` `**` and unary `-` accept numbers, and bool as int (`True + 1 == 2`). `str + str` concatenates. A string/number mix is a type error. `/` is true division; divide-by-zero raises `ZeroDivisionError`. `//` is floor division (`(-7) // 3 == -3`, `7.5 // 2 == 3`); divide-by-zero raises `ZeroDivisionError`. `%` is Python modulo (`(-7) % 3 == 2`). `**` uses binary64, so it does not grow arbitrary-precision integers. `0 ** 0` is `1`. A negative exponent of zero raises `ZeroDivisionError`. A negative base and a non-integer exponent, such as `(-2.0) ** 0.5`, is an unsupported numeric result: this subset has no complex value. An out-of-range index raises `IndexError`.

`raise` unwinds to the frame that owns the handler, so a helper `finally` runs before the error reaches the caller. A callback-local `try` does not unwind the caller.

New arithmetic, indexing, calls, and closure instructions require engine feature `lang.compute`. `engine_format_version` stays 1.

## Not supported

- `for` over a dict, `for`/`else`, `while/else`, `try/else`
- Classes, decorators, generators, async nested functions
- `map` / `filter` (they return iterators) and `functools.reduce` (it is an import). A nested function that takes a lambda and loops is the supported shape
- Collection truthiness (`if []:` / `if {}:`)
- Integers that are not exact binary64 values
- Mutable default literals and defaults that read another parameter
- `asyncio.gather`, `asyncio.wait`, `asyncio.FIRST_COMPLETED`, stored `gather` / `race`, and pre-awaited branch arguments (see [concurrency.md](concurrency.md))
- `*args`, keyword-only, positional-only, or a nested binding pattern
- Durable operations inside a helper (`effect`, `wait_for_event`, `sleep`, `invoke`, `gather`, `race`)
