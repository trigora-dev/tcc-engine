# Durable operations

Durable operations are explicit instructions. Ordinary computation is not journaled.

A library call is not a durable operation unless a frontend lowers it to one of the operations below, or the host exposes it as a capability.

For the TypeScript frontend, durable operations are **resolved imports** of `@tcc-engine/primitives` or `@trigora/sdk` exports `effect`, `waitForEvent`, `sleep`, and `invoke`. For the Python frontend, they are **resolved imports** of `tcc_engine.primitives` or `trigora` exports `effect`, `wait_for_event`, `sleep`, and `invoke`. Both spellings lower to the same instructions. Neither package is required at compile time: the frontend injects declarations. Aliased imports are durable. A function that is merely named `effect` is not. Durable operations are imports, not a context object or a `workflow()` wrapper.

The effect callback is not compiled into the artifact. The host performs the work identified by the string-literal key.

## Operations

| Operation | Instruction | Identity | Host request |
|---|---|---|---|
| Effect | `Effect` | `{execution_id}:{key}` | `run_effect`, then `persist_effect` |
| Sleep | `Sleep` | timer wait on this execution | `register_timer` |
| Event wait | `WaitForEvent` | `{execution_id}:{name}:{correlation_key}:{pc}` | `register_wait` |
| Child invoke | `Invoke` | `{parent}:invoke:{pc}` | `create_child` |

If an effect journal record is `completed`, a later `run_effect` for that identity must return the stored result and must not invoke the external operation again. The host looks up the journal by `{execution_id}:{key}` when the engine emits `run_effect`. After a crash the engine may emit `run_effect` again because in-memory outstanding requests are not part of the continuation.

A wake delivered to a wait or child that is already terminal is a no-op.

An absent event-wait correlation key is an empty field: `{execution_id}:{name}::{pc}`.

## Effects

Commit order:

1. Record effect start with a stable id and idempotency key.
2. Perform the external operation.
3. Persist the result.
4. Advance the continuation and persist the checkpoint.

If the process stops after step 2 and before step 3, the engine cannot determine whether the external operation occurred. Retry uses the same identity. This is not exactly-once external I/O.

Hosts should pass the idempotency key to providers that accept one. Ambiguous outcomes remain explicit in the protocol. A host that records `started` then crashes before `completed` may call the provider again with that key.

## Waits

The engine registers the wait, suspends the continuation, and yields to the host. The process may stop.

Resume occurs when the host delivers a matching timer, event, child completion, or cancellation.

A wait has exactly one terminal outcome: resolved, timed out, or cancelled. A wake delivered to a wait that is already resolved is a no-op. The host must not apply `event_payload` to a terminal wait.

A committed wait must have a durable wakeup registration.

## Child executions

A child is a separate execution: its own id, artifact, continuation, and journals.

The parent records a child wait and suspends. A logical invoke id creates at most one child. The child's terminal result is delivered to the parent at most once.

The engine assigns stable `invoke_id` and `child_execution_id` before the host writes. Those identities plus the parent wait checkpoint and child execution row are one logical transition: they must become durable together or not at all. Hosts may also group that unit with unrelated checkpoints in one commit. That coordination is optional host architecture; TCC correctness is still persist/confirm of a logical continuation.

## Cancellation

Cancellation is a host-delivered terminal transition. It takes effect at the next durable boundary. It does not interrupt an in-flight external operation.
