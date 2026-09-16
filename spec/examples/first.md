# First example

Hand-authored artifact for Phase 1. It is not produced by a compiler.

Corresponding TypeScript (sketch only; `frontends/typescript` does not compile it yet):

```ts
export default async function workflow(ctx) {
  const charged = await ctx.effect("charge", () => 42);
  await ctx.waitForEvent("approved");
  return charged;
}
```

Execution id: `first`. Artifact hash: `first-example`. One local. Language semantics: `ts.subset.v1`.

Required engine features: `ts.control_flow`, `durable.effect`, `durable.wait_for_event`.  
Required host capabilities: `host.persist_checkpoint`, `host.effect`, `host.event`.

## Instructions

| pc | Instruction |
|---|---|
| 0 | `LoadConst "charge"` |
| 1 | `Effect` |
| 2 | `StoreLocal 0` |
| 3 | `LoadConst "approved"` |
| 4 | `WaitForEvent` |
| 5 | `Pop` |
| 6 | `LoadLocal 0` |
| 7 | `Return` |

## Host sequence

In-memory progress is not a commit. `Ack` on `persist_checkpoint` leaves `revision` unchanged. `persist_confirmed` sets it.

Wait identity with no correlation key: `first:approved::4`.

Effect idempotency key: `first:charge`.

| Step | Engine outcome | Host response | Continuation after response |
|---|---|---|---|
| 1 | `run_effect` `{ key: charge, idempotency_key: first:charge }` | `effect_result` `42` | pc 2, stack `[42]`, pending none, `revision` still 0 |
| 2 | `persist_effect` completed `42` | `ack` | `persist_checkpoint` queued at `revision + 1` |
| 3 | `persist_checkpoint` | `persist_confirmed` `{ revision: 1 }` | `runnable`, `revision` 1 |
| 4 | (pure) store local, load `"approved"` | — | pc 4, locals `[42]` |
| 5 | `register_wait` `first:approved::4` | `ack` | `suspended`, pending event wait, persist queued |
| 6 | `persist_checkpoint` | `persist_confirmed` `{ revision: 2 }` | `suspended`, `revision` 2, pending wait |
| 7 | (suspended; no step) | `event_payload` `"ok"` | stack `["ok"]`, pending none, `runnable`, persist queued |
| 8 | `persist_checkpoint` | `persist_confirmed` `{ revision: 3 }` | `runnable`, `revision` 3 |
| 9 | `Pop`, `LoadLocal 0`, `Return` then `persist_checkpoint` | `persist_confirmed` `{ revision: 4 }` | `completed`, result `42` |

`budget == 0` before step 1 leaves pc 0 and `revision` 0.

If step 3 is answered with `ack` instead of `persist_confirmed`, `revision` stays 0.

Locked by `crates/tcc-core/tests/first_example.rs`.
