# Python subset

`language_semantics_version` `py.subset.v1`. A construct is supported only when it compiles, runs through the native Python binding, runs through WASM on the same artifact, and recovers from SIGKILL at every durable boundary with the same observable result.

Authoring uses resolved imports from `tcc_engine.primitives` (engine-native) or `trigora` (Trigora product). Both lower to the same durable operations. There is no `ctx` and no `workflow()` wrapper.

## Supported

- A single top-level `async def run()` with no parameters
- Imports from `tcc_engine.primitives` or `trigora`: `effect`, `wait_for_event`, `sleep`, `invoke` (aliases included)
- Assignment to simple locals
- `if` / `elif` / `else`
- `while`
- Unlabeled `break` / `continue`
- Nested blocks; names map to function-level slots
- `return` of a lowered expression
- `raise Exception(...)`, `try` / `except` / `finally`
- Literals: `None`, booleans, numbers that are exact IEEE-754 binary64 values, strings
- Dicts with string keys and lists (no spread, no cycles, no identity)
- Property read `obj.key` or `obj["key"]`
- `==` / `!=`, `is None` / `is not None`, unary `not`, numeric `<` `<=` `>` `>=`
- Sequential and branched mixes of durable operations

`if` / `while` / `not` operands must be bool, number, string, `None`, or a comparison. Collection literals used as conditions are compile errors. Effect keys, event names, and invoke names are string literals. Effect callbacks are not compiled into the artifact.

## Identities

- Effect: `{execution_id}:{key}`. Re-executing the same instruction with the same key skips a completed journal entry.
- Wait: `{execution_id}:{name}:{correlation}:{pc}` (empty correlation is an empty field)
- Child invoke: `{parent}:invoke:{pc}`

## Not supported

- `ctx.effect` / `workflow()` (not the authoring model)
- `for` / `for-in` / `for-else` / `while/else` / `try/else`
- Closures, nested functions, classes
- Arithmetic, `and` / `or` (use nested `if`)
- Collection truthiness (`if []:` / `if {}:`)
- Integers that are not exact binary64 values
- `Promise.all`-style fan-out
- Parameters on the entry function
