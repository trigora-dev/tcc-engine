# Execution semantics

The engine executes a validated artifact against a continuation. It does not own storage, networking, timers, or process lifecycle. Those belong to the host.

## Step loop

1. Load a validated artifact and a continuation.
2. Read the instruction at the top frame's program counter.
3. Execute pure instructions in the engine (`Jump`, locals, constants, `Return`).
4. At a durable operation, emit a `HostRequest` and stop.
5. Apply the host's `HostResponse`, then continue or suspend.

Host requests are issued at durable boundaries, not for every instruction. An in-memory transition is not committed until the host confirms the corresponding persist request.

`Engine::run_until_host(budget)` executes at most `budget` instructions. `budget == 0` executes none. Exhausting the budget returns `BudgetExhausted` without changing durability.

## Recovery

Recovery loads the latest committed continuation for an execution and resumes at its saved frames, program counter, and live values.

Completed instructions are not re-executed as a means of reconstructing that state. Execution history, if a host retains it, is not an input to recovery.

A continuation resumes only with the artifact identified by `artifact.hash`. See [Compatibility](compatibility.md).

## TypeScript subset

For `language_semantics_version` `ts.subset.v1`, `JumpIfTrue` and `JumpIfFalse` use JavaScript truthiness. The following values are falsy: `undefined`, `null`, `false`, `+0`, `-0`, `NaN`, and `""`. All other current value types are truthy.

A different language must not reuse these instructions for a different truthiness rule without a new engine feature.

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

The hand-authored first example, including expected host requests and continuation snapshots, is [First example](examples/first.md).

## Status

The stepper implements `Nop`, jumps, locals, constants, `Pop`, `Return`, durable-operation yield, and event-wait wake via `event_payload`. `Call` and exception instructions are not executed.
