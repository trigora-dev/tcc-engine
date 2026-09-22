# Continuation format

A continuation is the explicit program state required to resume an execution. It is not a native heap dump, a Rust object graph, or a copy of WebAssembly linear memory.

## Fields

| Field | Description |
|---|---|
| `execution_id` | Execution this continuation belongs to |
| `artifact.hash` | Artifact the continuation was produced from |
| `engine_format_version` | Continuation encoding version |
| `language_semantics_version` | Language rules in force when the continuation was created |
| `revision` | Host-confirmed commit number |
| `status` | `runnable`, `running`, `suspended`, `completed`, `failed`, or `cancelled` |
| `frames` | Call stack: function id, program counter, locals |
| `stack` | Operand stack |
| `pending` | Outstanding durable operation, if any. Still singular inside a join: the in-flight registration, not the whole group |
| `result` | Result value when status is `completed` |
| `try_stack` | Active exception handlers (`catch` pc, optional `finally` pc, stack depth). Absent or empty when none. |
| `join` | Active `JoinState` when a concurrent group is open. Absent or null when none. See [concurrency.md](concurrency.md) |
| `reentries` | Per-`Fork`-site next reentry count. Absent or empty when the execution has not finished a join |

Unknown continuation fields fail decode. They must not be dropped. A `join` object whose `kind`, `state`, `phase`, or branch `op` tag is unknown fails decode. `kind` is required. Missing `kind` fails decode.

Values in frames and on the stack use the encoding defined for the continuation's `language_semantics_version`. For `ts.subset.v1` that is `undefined`, `null`, boolean, IEEE-754 binary64 number, string, objects with string keys, and arrays (no cycles).

Unknown value tags must fail decode. They must not be coerced.

## Artifact identity

Resume must fail if `artifact.hash` does not match the artifact being loaded. The engine does not rewrite a continuation onto a different artifact.

## Durability

A continuation is committed when the host confirms `persist_checkpoint` (or an equivalent acknowledgement) for that revision. Unconfirmed in-memory updates are not recoverable state.

If a host reconstructs continuation bytes from a log or deltas, the result must be a continuation in this format. That reconstruction is not execution of completed program instructions.

The engine may emit semantic continuation deltas as persist intent on `persist_checkpoint`. Those deltas are not a continuation encoding and are not an `engine_format_version` change. Resume still consumes a full continuation in this format. Golden reconstruct vectors are in `spec/fixtures/persist/`.

Committed frames keep function-level slot topology: `locals.len()` equals the function's `local_count`. At persist the engine sets destination-dead slots to `undefined` in place. A later snapshot or omitted delta slot cannot resurrect a compacted payload.

Delta omission means the slot is unchanged. A `LocalPatch` to `{ t: "undefined" }` means the slot ceased to be live (or was assigned `undefined`). Omission is not a tombstone.

Recovery uses the latest committed continuation for the execution. Resume is `Engine::resume` with that continuation and the artifact whose hash matches `artifact.hash`.

## Encoding

In this repository, `tcc_state::encode_continuation` and `tcc_state::decode_continuation` serialize this format as tagged JSON. `encode_value` / `decode_value` serialize individual values. Decode of an unknown value tag returns `UnsupportedValue`.
