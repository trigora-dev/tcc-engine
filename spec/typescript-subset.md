# TypeScript subset

`language_semantics_version` `ts.subset.v1`. A construct is supported only when it compiles, runs natively and through WASM, and recovers from SIGKILL at every durable boundary with the same observable result.

Authoring uses resolved imports from `@tcc-engine/primitives` (engine-native) or `@trigora/sdk` (Trigora product). Both lower to the same durable operations. There is no `ctx` and no `workflow()` wrapper.

## Supported

- Default-export async function `run(a, b, ...)` with plain identifier parameters. A missing argument or `undefined` is replaced by a call-time default that may use earlier parameters only (`function run(a, b = a)`). Otherwise a short argument vector leaves the remaining parameters `undefined`. Extra arguments are ignored. Explicit `null` stays `null`. Rest and optional `?` stay unsupported
- Top-level non-async helpers with plain parameters. They may call other helpers. They cannot await a durable operation. A durable boundary runs only while the program entry frame is active, so a helper or closure frame is never live at a checkpoint. A top-level helper used as a value is one empty-environment closure for the execution, so `double === double`
- Imports from `@tcc-engine/primitives` or `@trigora/sdk`: `effect`, `waitForEvent`, `sleep`, `invoke` (aliases included)
- `const` / `let` with a single identifier and an initializer
- Assignment to `let` locals
- `if` / `else` / `else if`
- `while` and a simple `for` (`init; cond; assignment`)
- Unlabeled `break` / `continue`
- Nested blocks; names map to function-level slots
- `return` of a lowered expression
- `throw`, `try` / `catch` / `finally`
- Literals: `undefined`, `null`, booleans, finite numbers, strings. `NaN`, `Infinity`, `-Infinity`, and `-0` are produced by arithmetic and round-trip in the continuation encoding
- Objects and arrays as continuation-heap refs (no spread). Assignment copies the ref. Mutation is in place, including through a helper argument. Cycles are representable in the heap and cannot be inlined across a host boundary
- Property read and write `obj.key`, index read and write `xs[i]`, `xs.length`, `xs.push(value)`
- One-level array and object destructuring into identifiers. `const [a, b] = await Promise.all(...)` remains supported. No rest, spread, or nested patterns
- `===` / `!==` (reference equality on objects, arrays, and closures; `NaN === NaN` is false; `+0 === -0` is true), unary `!` and unary `-`, numeric `<` `<=` `>` `>=`
- `+` `-` `*` `/` `%` `**` and compound assignment, including `**=`, inside the operator domain below. `&&` and `||` short-circuit and return the operand
- Arrow functions with plain parameters, expression or block body, nested in `run`, a helper, or another arrow. No `async`, `this`, rest, or `arguments`. An arrow expression allocates a new closure each time it runs. A captured `let`, `const`, or parameter is one heap cell shared by the defining activation and every closure. Assignment to a captured `const` is a compile error. A closure value may live across `await`; its frame may not
- `xs.map`, `xs.filter`, and `xs.reduce` lower to the same index loop as `for...of`, then a closure call. `map` and `filter` call `(element, index, array)`. `reduce` calls `(accumulator, element, index, array)`. Extra parameters are ignored. `filter` uses ordinary JavaScript truthiness. `reduce(fn)` and `reduce(fn, undefined)` differ because the argument is present or absent: `[].reduce(fn)` throws, and `[].reduce(fn, undefined)` returns `undefined`. Length is captured once. Rejecting `xs.push` inside `xs.map`, including through an alias, is a subset restriction stricter than JavaScript. Mutating a distinct element is allowed. `thisArg` is a compile error. A non-function callback is a runtime type error
- `cond ? a : b`
- `for (const x of xs)` over an array. Length is captured once. The index is a real local. Structural mutation of the iterated cell, including through an alias, is a runtime error. Mutating a distinct element is allowed. `for-in` stays unsupported
- Sequential and branched mixes of durable operations
- Direct `await Promise.all([effect | waitForEvent | sleep | invoke, ...])` and `await Promise.race([...])` of at most 32 branches, including `return await` of either and an outer use of that await. Array destructuring is supported only for `await Promise.all(...)`. See [concurrency.md](concurrency.md)
- `invoke(name, ...args)`. Each argument after the name is a subset value. See [durable-operations.md](durable-operations.md)

Effect keys, event names, and invoke names are string literals. Effect callbacks are not compiled into the artifact.

## Identities

- Effect: `{execution_id}:{key}`. Re-executing the same instruction with the same key skips a completed journal entry.
- Wait: `{execution_id}:{name}:{correlation}:{pc}` (empty correlation is an empty field)
- Child invoke: `{parent}:invoke:{pc}` outside a join. Inside `Promise.all` or `Promise.race`, wait, timer, and child ids are the branch id from [concurrency.md](concurrency.md).

`+` is `number + number`, or string concatenation with `string`, `number`, `bool`, `null`, or `undefined` (`null` becomes `"null"`, `undefined` becomes `"undefined"`). `-` `*` `/` `%` `**` and unary `-` require numbers. `/` is IEEE (`1 / 0` is `Infinity`, `0 / 0` is `NaN`). `%` is JavaScript remainder. `**` follows JavaScript, including `0 ** 0 === 1`, `0 ** -1 === Infinity`, `(-0) ** -1 === -Infinity`, `(-0) ** 3 === -0`, `Infinity ** 0 === 1`, `NaN ** 0 === 1`, and `(-2) ** 0.5 === NaN`. `true + 1`, `null + 1`, and `"3" - 1` are runtime type errors. Indexing out of range, including a negative index, is `undefined`.

`throw` unwinds to the frame that owns the handler, so a helper `finally` runs before the error reaches the caller. A callback-local `try` does not unwind the caller.

New arithmetic, indexing, calls, and closure instructions require engine feature `lang.compute`. `engine_format_version` stays 1.

## Not supported

- Labeled `break` / `continue`, `for-in`, `for await`
- `this`, classes, prototypes, generators, async arrows
- `++` / `--`, template literals, optional chaining, `delete`, `new`. JavaScript has no `//`
- collection `ToPrimitive`, string-to-number coercion
- Destructuring with rest, spread, or nested patterns
- Stored promises, `Promise.any` / `Promise.allSettled`, spread or dynamic `Promise.all` / `Promise.race` arguments
- Rest parameters, optional `?` parameters, and defaults that read a later parameter or a local
- Durable operations inside a helper (`effect`, `waitForEvent`, `sleep`, `invoke`, `Promise.all`, `Promise.race`)
