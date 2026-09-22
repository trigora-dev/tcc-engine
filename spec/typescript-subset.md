# TypeScript subset

`language_semantics_version` `ts.subset.v1`. A construct is supported only when it compiles, runs natively and through WASM, and recovers from SIGKILL at every durable boundary with the same observable result.

Authoring uses resolved imports from `@tcc-engine/primitives` (engine-native) or `@trigora/sdk` (Trigora product). Both lower to the same durable operations. There is no `ctx` and no `workflow()` wrapper.

## Supported

- Default-export async function with no parameters
- Imports from `@tcc-engine/primitives` or `@trigora/sdk`: `effect`, `waitForEvent`, `sleep`, `invoke` (aliases included)
- `const` / `let` with a single identifier and an initializer
- Assignment to `let` locals
- `if` / `else` / `else if`
- `while` and a simple `for` (`init; cond; assignment`)
- Unlabeled `break` / `continue`
- Nested blocks; names map to function-level slots
- `return` of a lowered expression
- `throw`, `try` / `catch` / `finally`
- Literals: `undefined`, `null`, booleans, numbers, strings
- Objects and arrays (no spread, no cycles, no identity)
- Property read `obj.key`
- `===` / `!==`, unary `!`, numeric `<` `<=` `>` `>=`
- Sequential and branched mixes of durable operations
- Direct `await Promise.all([effect | waitForEvent | sleep | invoke, ...])` and `await Promise.race([...])` of at most 32 branches, including `return await` of either and an outer use of that await. Array destructuring is supported only for `await Promise.all(...)`. See [concurrency.md](concurrency.md).

Effect keys, event names, and invoke names are string literals. Effect callbacks are not compiled into the artifact.

## Identities

- Effect: `{execution_id}:{key}`. Re-executing the same instruction with the same key skips a completed journal entry.
- Wait: `{execution_id}:{name}:{correlation}:{pc}` (empty correlation is an empty field)
- Child invoke: `{parent}:invoke:{pc}` outside a join. Inside `Promise.all` or `Promise.race`, wait, timer, and child ids are the branch id from [concurrency.md](concurrency.md).

## Not supported

- `ctx.effect` / `workflow()` (not the authoring model)
- Labeled `break` / `continue`, `for-in` / `for-of`
- Closures, nested functions, `this`, classes
- `++` / `--`, arithmetic, logical `&&` / `||` (use nested `if`)
- Destructuring other than `const [a, b] = await Promise.all(...)`, rest/spread, optional chaining, `new`
- Stored promises, `Promise.any` / `Promise.allSettled`, spread or dynamic `Promise.all` / `Promise.race` arguments
- Parameters on the entry function
