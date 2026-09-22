# Execution semantics

The engine executes a validated artifact against a continuation. It does not own storage, networking, timers, or process lifecycle. Those belong to the host.

## Step loop

1. Load a validated artifact and a continuation.
2. Read the instruction at the top frame's program counter.
3. Execute pure instructions in the engine (`Jump`, locals, constants, `Return`).
4. At a durable operation, emit a `HostRequest` and stop. `Fork` / `JoinAll` / `JoinAny` ([concurrency.md](concurrency.md)) register branches one durable transition at a time. `JoinAll` suspends while any branch is still outstanding. `JoinAny` registers every branch before it settles.
5. Apply the host's `HostResponse`, then continue or suspend. A delivery into a concurrent group must name `branch`.

Host requests are issued at durable boundaries, not for every instruction. An in-memory transition is not committed until the host confirms the corresponding persist request.

`Engine::run_until_host(budget)` executes at most `budget` instructions. `budget == 0` executes none. Exhausting the budget returns `BudgetExhausted` without changing durability.

## Recovery

Recovery loads the latest committed continuation for an execution and resumes at its saved frames, program counter, and live values.

Completed instructions are not re-executed as a means of reconstructing that state. Execution history, if a host retains it, is not an input to recovery.

A continuation resumes only with the artifact identified by `artifact.hash`. See [Compatibility](compatibility.md).

## TypeScript subset

For `language_semantics_version` `ts.subset.v1`, `JumpIfTrue` and `JumpIfFalse` use JavaScript truthiness. The following values are falsy: `undefined`, `null`, `false`, `+0`, `-0`, `NaN`, and `""`. Objects and arrays are truthy. All other current value types are truthy.

A construct is supported only when compile, native execution, WASM execution, and SIGKILL/resume at every durable boundary agree. Untaken branches and completed journaled effects must not run after resume. The supported source subset is [TypeScript subset](typescript-subset.md).

Same-key effects and same-pc waits keep frozen identities (`{execution_id}:{key}` and `{execution_id}:{name}:{correlation}:{pc}`). A loop that re-executes the same `Effect` instruction therefore hits the journal skip path rather than a new provider call.

A different language must not reuse these instructions for a different truthiness rule without a new engine feature.

## Python subset

For `language_semantics_version` `py.subset.v1`, the frontend maps Python onto the same instructions and value tags. `None` is `null`. Collection truthiness is not compiled (`if []` / `if {}` are errors) so `JumpIf*` keeps JavaScript truthiness for bool, number, string, and `null`. Integers that are not exact IEEE-754 binary64 values are compile errors.

A construct is supported only when compile, native PyO3 execution, WASM execution of the same artifact, and SIGKILL/resume at every durable boundary agree. The supported source subset is [Python subset](python-subset.md). The first example is [First example (Python)](examples/first-python.md).

## Statuses

| Status | Meaning |
|---|---|
| `runnable` | May execute |
| `running` | Currently executing a segment |
| `suspended` | Waiting for the host (timer, event, child, or equivalent) |
| `completed` | Terminal success |
| `failed` | Terminal failure |
| `cancelled` | Terminal cancellation |

`completed`, `failed`, and `cancelled` are terminal. Resuming a terminal continuation is an error.

A suspended execution does not proceed until the host delivers the matching wake. Wake delivery is host-driven.

The first example, including expected host requests and continuation snapshots, is [First example](examples/first.md).

## Implemented instructions

The stepper implements `Nop`, jumps, locals, constants, `Pop`, `NewObject`, `SetProp`, `GetProp`, `NewArray`, `ArrayPush`, `StrictEq` / `StrictNeq`, numeric compare, `Not`, `Return`, `Throw` / `PushTry` / `PopTry`, durable-operation yield, event-wait wake via `event_payload`, timer wake via `timer_fired`, child wake via `child_result`, and host `cancel`. `Call` is defined in the format and is not executed.
