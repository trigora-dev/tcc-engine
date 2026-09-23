# Structured concurrency

`Promise.all` and `Promise.race` are durable control-flow constructs, not syntax sugar over JavaScript promises. The continuation stores a `JoinState`. It does not store host `Promise` objects.

Python uses the same joins through compiler intrinsics, not through `asyncio`:

```text
TypeScript Promise.all / Promise.race
Python    gather / race
Rust      join / race, exactly two direct durable calls
All three Fork + JoinAll / JoinAny, feature durable.concurrent_group
```

The engine cap is unchanged: 1 through 32 branches. Rust's two-argument spelling is a frontend limit. Rust has no varargs, and this subset has no varargs macro, so `join(a, b)` and `race(a, b)` are the v1 calls. A later spelling can pass more branches through without an engine change. `join` yields `Vec<T>` in input order. `race` yields the winning `T`. Both branches must share `T`. The calls are direct durable calls. They are not stored.

`gather` and `race` are not runtime coroutine helpers. They cannot be passed around as ordinary functions. `asyncio.gather`, `asyncio.wait`, and `asyncio.FIRST_COMPLETED` are not lowered. `Promise.any` and `Promise.allSettled` are not specified as executable operations.

## Grammar

Only these shapes:

```text
AwaitExpression(
  CallExpression(Promise.all | Promise.race, ArrayLiteral of durable expressions)
)
```

In: `await Promise.all([...])`, `await Promise.race([...])`, `return await` of either, and an outer use such as `foo(await Promise.all([...]))` or `foo(await Promise.race([...]))`. For `Promise.all`, the engine materializes the aggregate in **input order** before that outer expression runs. For `Promise.race`, the engine materializes the winning value before that outer expression runs.

Out: a stored promise (`const p = Promise.all([...]); await p`, and the same for `Promise.race`), `return Promise.all([...])` or `return Promise.race([...])` without `await`, spread, dynamic arrays, `.then`, arbitrary `Promise` values, `Promise.any`, `Promise.allSettled`, and non-durable calls.

Branch expressions are `effect`, `waitForEvent`, `sleep`, and `invoke`.

Python, imported from `tcc_engine.primitives` or `trigora` (aliases included):

```text
Await(
  Call(gather | race, positional direct durable calls)
)
```

```python
from tcc_engine.primitives import effect, wait_for_event, sleep, invoke, gather, race
```

`return await gather(...)` is in. Out: `return gather(...)`, `x = gather(...)`, `foo(gather(...))`, and `pending = gather(...); await pending`. Each argument is a direct call to `effect`, `wait_for_event`, `sleep`, or `invoke`. `gather(await effect(...), ...)` is rejected, because that call would finish before `Fork`. Nested `gather` / `race` is rejected. A locally defined function named `gather` is not durable. No `*args`, keywords, or list wrapper. 1–32 arguments.

At most **32** branches. The TypeScript compiler, the Python compiler, and artifact validation all reject a larger group. A hand-built artifact cannot bypass the cap.

## Instructions

`Fork` creates one `JoinInstance` with N ordered branch descriptors. The instruction at `join_pc` sets the join kind: `JoinAll` is `all`, `JoinAny` is `any`. `JoinAll` waits until every branch has completed and pushes the aggregate array in input order. `JoinAny` pushes the winning branch's value.

`pending` stays the single in-flight host request. The join is separate control state.

`engine_format_version` stays `1`. Format v1 includes `Fork`, `JoinAll`, and `JoinAny`. Artifacts that use them require `durable.concurrent_group`. That feature is a capability inside the format. It is not a promise to keep executing a pre-join interpretation of internal artifacts. Unknown opcodes fail closed. A continuation `join` the engine does not implement fails closed. Decoders must not ignore unknown continuation fields. `kind` is required on every join. Missing `kind` fails decode. There is no "missing means all."

## Identities

```text
join_instance_id = execution_id + join_site_id + reentry
branch_id        = join_instance_id + branch_index
```

`join_site_id` is the `Fork` program counter. `reentry` increments only when that join reaches `succeeded` or `failed` and the site runs again. Branch index is source order, never completion order.

The same `branch_id` correlates host registrations, continuation state, event delivery, timer firing, child edges, and error delivery.

Effect journal identity stays `{execution_id}:{key}`. A repeated key still journal-skips, including across loop iterations. The branch still has a `branch_id` when the journal key does not mention it.

Invoke, wait, and timer ids on the wire are the `branch_id`, not `{parent}:invoke:{pc}` alone. That pc-only form aliases across loop iterations.

## Branch phases

`planned | registered | completed | failed | detached`

```text
persist membership as planned
→ emit the host request
→ host durably registers it
→ persist phase = registered
→ CRASH
```

Resume retries a `planned` branch. A `registered` branch is never re-registered blindly: recover it from host state under `branch_id`, or wait for the correlated delivery. Every registration request is retry-safe under `branch_id`.

Registration is sequential, one durable transition at a time. Results are still one aggregate in input order. Host work that is already registered (a wait, a timer, a child) may complete later, and those completions may arrive out of source order.

## Join outcome

```text
JoinState {
  kind: all | any
  site, reentry,
  state: active | succeeded | failed,
  winner_branch?,
  failure_branch?,
  branches: [{ index, branch_id, op, phase, result?, error? }]
}
```

`JoinAll` suspends while `state = active` and any branch is not `completed`. The aggregate is input order. `winner_branch` stays empty for `all`.

Fail-fast: persist `state = failed` and `failure_branch` **before** acknowledging the completion that caused it. After that commit:

- Later sibling completions cannot change the outcome.
- Stale wait, timer, and child deliveries are ignored.
- Already-started effects may finish externally.
- Already-created children may keep running.
- The parent throws into the existing `try` / `catch` exactly once.

**Detach** means the branch can no longer affect the parent join. It does not cancel the underlying effect or child. This slice has no unregister or child-cancel protocol.

## Events and timers

One event delivery resolves **one** matching durable wait, in registration / source order. It does not broadcast. Two `waitForEvent("ready")` branches are two waits. Two `sleep` calls with the same deadline are two timers.

## Race

`Promise.race` registers every branch in source order before it may settle. A still-`planned` branch is started. The race does not skip it because an earlier branch already has a result.

Registration is still one durable transition at a time. While any branch is `planned`, `state` stays `active` and `winner_branch` stays empty. An effect that finishes during this pass stores its result or error on that branch and does not settle the race. When no branch is `planned`, if any branch has already completed or failed, one settle checkpoint picks the winner. Otherwise the join suspends until a correlated delivery, then settles on that delivery.

The winner is the first completion durably accepted after registration, not wall-clock order. The settle checkpoint records `winner_branch`, terminal `state`, and that branch's `result` or `error` together. Crash before that confirm can still pick a different winner. Crash after it cannot. Resume pushes the stored result, or throws the stored error, exactly once. The parent does not continue until that checkpoint is confirmed.

Several effects can finish during the registration pass. The settle checkpoint then applies the tie rule: the lowest branch index wins. The reference host delivers one wake per transition, so that tie is an engine rule. A higher index still wins when it is the first delivery after registration. That is a later transition, not a tie.

A successful winner pushes that branch's value, not an array, and steps past `JoinAny`. A rejecting winner sets both `winner_branch` and `failure_branch` to that index, then throws into the existing `try` / `catch` exactly once.

After the winner checkpoint is confirmed, every other `registered` branch is `detached`. Detach does not cancel the underlying effect or child. An effect that already ran may finish externally. An already-created child keeps running.

```ts
await Promise.race([
  effect("charge", async () => chargeCard()),
  sleep(5_000),
])
```

Both branches start. If the timer wins, that does not mean the charge effect did not run.

`Promise.all` does not use this settle rule. It still fail-fasts without waiting for later siblings, and it still waits for every branch on success.

## Liveness

At persist, destination-dead slots are `undefined`.

- A local unused after the join is dead across it.
- A completed branch result stays live while any sibling is outstanding and `JoinAll` still needs that result.
- After `const [a, b] = await Promise.all(...)`, if only `a` is used later, `b` is dead at the next durable boundary.

## Host deliveries

Finalized host protocol v1. There is no protocol v2 and no decoder for the pre-correlation payload.

```text
target:
  execution
  branch?     # required when the delivery is into a concurrent group
```

Non-join executions omit `branch`. A join delivery without `branch` is rejected. Missing does not mean branch 0.

Wait, timer, and child deliveries into a join must carry `branch`. Effect journal rows stay `{execution_id}:{key}` and are not branch-addressed.
