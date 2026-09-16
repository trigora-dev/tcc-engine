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

Durable operations:

- `Effect`
- `Sleep`
- `WaitForEvent`
- `Invoke`

Exceptions:

- `Throw`
- `PushTry { catch, finally }`
- `PopTry`

Constants are `undefined`, `null`, boolean, IEEE-754 binary64 number, and string. These are JavaScript value semantics. They are not a universal language value model.

## Language semantics

Instruction meaning is bound to `language_semantics_version`. Sharing an encoding does not make language rules identical.

A frontend that needs different numeric types, truthiness, exceptions, or object identity must introduce new instructions or runtime helpers and declare a new required engine feature. Existing artifacts must not change meaning when the format is extended.

## Validation

Validation must run before execution. It checks:

- `engine_format_version`
- non-empty `artifact_hash`
- required engine features and host capabilities against the implementing engine and host
- non-empty program
- unique function ids
- entry function present
- jump targets in range
- span array length equal to instruction count
- call targets present

In this repository, validation is `tcc_ir::validate`.

## Status

Format version `1` is the TypeScript subset identified by `language_semantics_version` `ts.subset.v1`. Control, locals, constants, and durable-operation instructions are defined. Exception instructions are encoded in the format; execution of `Call`, `Throw`, `PushTry`, and `PopTry` is not implemented.
