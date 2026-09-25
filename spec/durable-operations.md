# Durable operations

Durable operations are explicit instructions. Ordinary computation is not journaled.

A library call is not a durable operation unless a frontend lowers it to one of the operations below, or the host exposes it as a capability. `await Promise.all([...])` / `await Promise.race([...])` and `await gather(...)` / `await race(...)` of those operations are structured concurrency in [concurrency.md](concurrency.md), not extra host operations.

For the TypeScript frontend, durable operations are **resolved imports** of `@tcc-engine/primitives` or `@trigora/sdk` exports `effect`, `waitForEvent`, `sleep`, and `invoke`. For the Python frontend, they are **resolved imports** of `tcc_engine.primitives` or `trigora` exports `effect`, `wait_for_event`, `sleep`, `invoke`, `gather`, and `race`. `gather` and `race` are compiler intrinsics: `gather` lowers to `JoinAll` and `race` lowers to `JoinAny`. Both spellings lower to the same instructions. Neither package is required at compile time: the frontend injects declarations. For the Rust frontend, they are **resolved imports** of `trigora` or `tcc_rust_prelude` exports `effect`, `sleep`, `wait_for_event`, `invoke`, `join`, and `race`. `use trigora::{effect, sleep}` and `use trigora::effect as fx` are durable. `use trigora::*` is rejected. A qualified call such as `trigora::effect(...)` is rejected. A helper, parameter, local, or pattern that shadows an imported durable name is rejected. Aliased imports are durable. A function that is merely named `effect`, `gather`, or `race` and was not imported is not. Durable operations are imports, not a context object or a `workflow()` wrapper.

The effect callback is not compiled into the artifact. The host performs the work identified by the string-literal key. `Effect` pops the key. When `has_input` is true it pops one value first. When `has_input` is false, or the field is omitted, it pops nothing and the host input is `{}`. TypeScript and Python encode `{"op":"Effect"}`. Rust always encodes `{"op":"Effect","has_input":true}`, including a closure that captures nothing. Both reach the host as `{}` when there is no payload. Names on a Rust effect object are a frontend ABI between the Rust lowerer and the effect harness. They are not TCC object semantics and not a stable engine layout. See [rust-subset.md](rust-subset.md).

## Operations

| Operation | Instruction | Identity | Host request |
|---|---|---|---|
| Effect | `Effect` | `{execution_id}:{key}` | `run_effect`, then `persist_effect` |
| Sleep | `Sleep` | timer wait on this execution | `register_timer` |
| Event wait | `WaitForEvent` | `{execution_id}:{name}:{correlation_key}:{pc}` | `register_wait` |
| Child invoke | `Invoke` | `{parent}:invoke:{pc}` outside a join; `branch_id` inside one | `create_child` |
| Concurrent group | `Fork` / `JoinAll` / `JoinAny` | `branch_id = execution + Fork pc + reentry + index` | the branch's own request, correlated by `branch` |

If an effect journal record is `completed` and the canonical input matches, a later `run_effect` for that identity must return the stored result and must not invoke the external operation again. The same identity with a different canonical input is an invariant error, including a `started` row, and must not reuse the stored result. Comparison uses the canonical value, not raw JSON text. The journal column is non-null. A no-payload effect stores `{}`. Object key order is not part of equality. The host looks up the journal by `{execution_id}:{key}` when the engine emits `run_effect`. The input is not part of that identity and is not stored on the continuation. After a crash the engine may emit `run_effect` again because in-memory outstanding requests are not part of the continuation. A joined effect uses that same `run_effect` shape, with that branch's own input, rebuilt at the yield. `run_effect` always includes `input`. A missing field is `missing run_effect.input`.

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

`invoke(name, ...args)` supplies an ordered argument vector. `invoke(name)` supplies `[]`. An explicit `null` / `None` is one element, not an empty vector. The caller supplies that vector. The invoked program's `language_semantics_version` determines how it binds to the declared parameters, into slots `0..param_count-1`. Binding runs on a fresh start only. Resume restores the continuation and does not bind arguments again. After binding, each slot is an ordinary local: a checkpoint keeps it only while a later instruction still needs it.

```ts
export default async function run(input) {
  const analysis = await invoke("analyze", { sources: input.sources });
  return analysis;
}
```

```ts
export default async function run(a, b) {
  const analysis = await invoke("analyze", a, b);
  return analysis;
}
```

```python
from tcc_engine.primitives import program, invoke

@program
async def research(input):
    analysis = await invoke("analyze", {"sources": input["sources"]})
    return analysis
```

```python
from tcc_engine.primitives import program, invoke

@program
async def research(a, b):
    analysis = await invoke("analyze", a, b)
    return analysis
```

A TypeScript caller may pass a shorter vector than a TypeScript callee declares; the remaining parameters are `undefined`. The same short vector fails when the callee is Python. Arity follows the callee artifact, not the caller, including when the two programs use different languages.

The parent records a child wait and suspends. A logical invoke id creates at most one child and permanently binds that id to the first committed argument vector. A later create with a different vector does not replace it and surfaces an invariant mismatch. The child's terminal result is delivered to the parent at most once.

The engine assigns stable `invoke_id` and `child_execution_id` before the host writes. Those identities plus the parent wait checkpoint and child execution row are one logical transition: they must become durable together or not at all. Hosts may also group that unit with unrelated checkpoints in one commit. That coordination is optional host architecture; TCC correctness is still persist/confirm of a logical continuation.

## Cancellation

Cancellation is a host-delivered terminal transition. It takes effect at the next durable boundary. It does not interrupt an in-flight external operation.
