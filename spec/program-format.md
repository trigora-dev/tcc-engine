# Program format

A TCC program is a versioned artifact. Language frontends emit this format. The engine validates an artifact before executing it.

Source text is not an input to the engine.

## Artifact

An artifact has an envelope and a program.

| Field | Description |
|---|---|
| `artifact_hash` | Identity recorded by continuations produced from this artifact |
| `frontend_id` | Frontend that produced the artifact |
| `frontend_version` | Version of that frontend |
| `language_semantics_version` | Language rules assumed by the instructions |
| `engine_format_version` | Envelope and instruction encoding. Current value: `1` |
| `required_engine_features` | Engine features the program uses |
| `required_host_capabilities` | Host capabilities the program uses |
| `runtime_modules` | Optional helper modules required at runtime |
| `program` | Functions, entry point, instructions, and source spans |

An engine must reject an artifact that requires an engine feature or host capability it does not implement. Ignoring a requirement is not valid execution.

## Program

A program contains one or more functions and an entry function id.

Each function has:

- a unique function id
- a name
- parameter and local counts. `param_count` is not greater than `local_count`. Slots `0..param_count-1` are the program parameters, in source order
- an instruction sequence
- a source-span array of the same length as the instruction sequence (`null` spans are allowed)

Jump and try targets must be in range for that function. Call targets must name a function that exists in the program. The entry function must exist. `program.entry` is that function's id. The function's `name` is source text for diagnostics. Execution identity is the id.

## Instructions

Format version `1` defines the following instructions.

Control:

- `Nop`
- `Jump { target }`
- `JumpIfTrue { target }`
- `JumpIfFalse { target }`
- `Call { func, argc }`
- `Return`

Locals and constants:

- `LoadLocal { local }`
- `StoreLocal { local }`
- `LoadConst { value }`
- `Pop`
- `NewObject`
- `SetProp { key }`
- `GetProp { key }`
- `NewArray`
- `ArrayPush`
- `ArrayIndex { index }` — copy one element of the array on top of the stack, leaving the array in place
- `StrictEq`
- `StrictNeq`
- `Lt` / `Le` / `Gt` / `Ge`
- `Not`

Durable operations:

- `Effect`. `has_input: false` encodes as `{"op":"Effect"}`; a missing field decodes as false. That omission is the encoding of false. `has_input: true` encodes as `{"op":"Effect","has_input":true}` and pops one value under the key, including `{}`. False pops only the key, and the host input is `{}`. A non-boolean `has_input` is invalid. TypeScript and Python emit `{"op":"Effect"}`. Rust always emits `{"op":"Effect","has_input":true}`
- `Sleep`
- `WaitForEvent`
- `Invoke`. Optional `arg_count`. Omitted or zero pops only the program name. A positive count pops that many values underneath the name, in reverse, and restores source order. Encode omits `arg_count` when it is zero; a missing field decodes as zero
- `Fork { count, join_pc }` / `JoinAll` / `JoinAny` — require `durable.concurrent_group`; `count` is 1 through 32. `JoinAny` is `Promise.race`. See [concurrency.md](concurrency.md)

Exceptions:

- `Throw`
- `PushTry { catch, finally }`
- `PopTry`

Constants are `undefined`, `null`, boolean, IEEE-754 binary64 number, and string. Finite numeric constants, including `-0`, are JSON numbers. Non-finite constants use `"NaN"`, `"Infinity"`, or `"-Infinity"` on the same `number` tag. Runtime values may also be objects (`NewObject` / `SetProp` / `GetProp`) and arrays (`NewArray` / `ArrayPush`). In a continuation those collections are heap refs, so aliasing and cycles are representable. TypeScript `===` / `!==` on collections is reference equality. Python `==` / `!=` on collections is structural. Numeric compare requires two numbers. These are not a universal language value model. Identity does not cross a host boundary: effect results, event payloads, and invoke arguments are inlined.

## Language semantics

Instruction meaning is bound to `language_semantics_version`. Sharing an encoding does not make language rules identical.

The caller supplies an ordered argument vector. The invoked program's `language_semantics_version` determines how that vector binds to its declared parameters. `Invoke` consumes exactly `arg_count` values under the program name (`arg_count` omitted means zero). Parameter slots are written on a fresh start only. Resume does not bind them again.

A frontend that needs different numeric types, truthiness, exceptions, or object identity must introduce new instructions or runtime helpers and declare a new required engine feature. Existing artifacts must not change meaning when the format is extended.

## Validation

Validation must run before execution. It checks:

- `engine_format_version`
- non-empty `artifact_hash`
- non-empty `frontend_id` (ids are not allowlisted)
- `language_semantics_version` in `{ ts.subset.v1, py.subset.v1, rust.subset.v1 }`
- required engine features and host capabilities against the implementing engine and host
- non-empty program
- unique function ids
- entry function present
- jump targets in range
- `LoadLocal` / `StoreLocal` indices `< local_count`
- span array length equal to instruction count
- call targets present
- size caps: at most 1024 functions, 100000 instructions per function, 4096 locals, and 1048576 bytes per const string or property key

Resume also checks that the continuation `engine_format_version` and `language_semantics_version` match the artifact, each frame’s `locals.len()` equals that function’s `local_count`, and `pc` is in range. Unknown opcodes fail at decode (`IrError::InvalidEncoding`), not as a panic. OOB locals fail validate; they are not `UnknownInstruction` at runtime.

In this repository, validation is `tcc_ir::validate`. `tcc_ir::encode_artifact` / `decode_artifact` serialize the envelope as JSON. `artifact_hash` is SHA-256 of the canonical JSON of the artifact with an empty `artifact_hash` field, hex-encoded. The frontend computes the hash; the engine validates the envelope and does not re-hash.

## Current coverage

Format version `1` is shared by `ts.subset.v1`, `py.subset.v1`, and `rust.subset.v1`. Control, locals, constants, objects, arrays, comparisons, exceptions, and durable-operation instructions are defined. `Call` is encoded in the format and is not executed. Supported source languages are [TypeScript subset](typescript-subset.md), [Python subset](python-subset.md), and [Rust subset](rust-subset.md). First examples: [TypeScript](examples/first.md), [Python](examples/first-python.md), [Rust](examples/first-rust.md).
