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
- parameter and local counts
- an instruction sequence
- a source-span array of the same length as the instruction sequence (`null` spans are allowed)

Jump and try targets must be in range for that function. Call targets must name a function that exists in the program. The entry function must exist.

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
- `StrictEq`
- `StrictNeq`
- `Lt` / `Le` / `Gt` / `Ge`
- `Not`

Durable operations:

- `Effect`
- `Sleep`
- `WaitForEvent`
- `Invoke`

Exceptions:

- `Throw`
- `PushTry { catch, finally }`
- `PopTry`

Constants are `undefined`, `null`, boolean, IEEE-754 binary64 number, and string. Runtime values may also be objects (`NewObject` / `SetProp` / `GetProp`) and arrays (`NewArray` / `ArrayPush`) whose elements are those value types. Nested identity and cycles are not in this format version’s executed subset. `===` / `!==` compare values structurally (there is no object identity). Numeric compare requires two numbers. These are JavaScript-inspired value semantics. They are not a universal language value model.

## Language semantics

Instruction meaning is bound to `language_semantics_version`. Sharing an encoding does not make language rules identical.

A frontend that needs different numeric types, truthiness, exceptions, or object identity must introduce new instructions or runtime helpers and declare a new required engine feature. Existing artifacts must not change meaning when the format is extended.

## Validation

Validation must run before execution. It checks:

- `engine_format_version`
- non-empty `artifact_hash`
- non-empty `frontend_id` (ids are not allowlisted)
- `language_semantics_version` in `{ ts.subset.v1, py.subset.v1 }`
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

Format version `1` is shared by `ts.subset.v1` and `py.subset.v1`. Control, locals, constants, objects, arrays, comparisons, exceptions, and durable-operation instructions are defined. `Call` is encoded in the format and is not executed. Supported source languages are [TypeScript subset](typescript-subset.md) and [Python subset](python-subset.md). First examples: [TypeScript](examples/first.md), [Python](examples/first-python.md).
