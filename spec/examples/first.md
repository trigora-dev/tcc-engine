# First example

A TypeScript program that imports durable operations from `@tcc-engine/primitives` and exports a default async function. There is no context object or workflow wrapper.

The TypeScript frontend treats `@tcc-engine/primitives` and `@trigora/sdk` as durable-operations modules. The frontend recognizes resolved imports of `effect` and `waitForEvent`. Programs that differ only in that specifier produce the same artifact hash. Aliasing works:

```ts
import { effect as durableEffect } from "@tcc-engine/primitives";
```

A locally declared `function effect` is not durable.

```ts
import { effect, waitForEvent } from "@tcc-engine/primitives";

export default async function run() {
  const result = await effect("generate", async () => {
    return generateSomething();
  });
  const approval = await waitForEvent("approved");
  return { result, approval };
}
```

The effect callback body is not compiled into the artifact. The host supplies the result for key `generate`. Keys and event names must be string literals. Unsupported constructs are compile errors.

Language semantics: `ts.subset.v1`. Required engine features: `ts.control_flow`, `durable.effect`, `durable.wait_for_event`. Required host capabilities: `host.persist_checkpoint`, `host.effect`, `host.event`.

Walkthrough execution id: `first`. Locals: `result` = 0, `approval` = 1.

## Instructions

| pc | Instruction |
|---|---|
| 0 | `LoadConst "generate"` |
| 1 | `Effect` |
| 2 | `StoreLocal 0` |
| 3 | `LoadConst "approved"` |
| 4 | `WaitForEvent` |
| 5 | `StoreLocal 1` |
| 6 | `NewObject` |
| 7 | `LoadLocal 0` |
| 8 | `SetProp "result"` |
| 9 | `LoadLocal 1` |
| 10 | `SetProp "approval"` |
| 11 | `Return` |

`SetProp { key }` pops a value, then an object, writes `object[key] = value`, and pushes the object.

## Host sequence

In-memory progress is not a commit. `Ack` on `persist_checkpoint` leaves `revision` unchanged. `persist_confirmed` sets it.

Wait identity with no correlation key: `first:approved::4`.  
Effect idempotency key: `first:generate`.

With a host that answers effect `generate` with `42` and delivers event payload `"ok"`, the completed result is `{ result: 42, approval: "ok" }`.

| Step | Engine outcome | Host response | Continuation after response |
|---|---|---|---|
| 1 | `run_effect` `{ key: generate, idempotency_key: first:generate }` | `effect_result` `42` | pc 2, stack `[42]`, `revision` 0 |
| 2 | `persist_effect` | `ack` | persist queued |
| 3 | `persist_checkpoint` | `persist_confirmed` `{ revision: 1 }` | `runnable`, `revision` 1 |
| 4 | store local, load `"approved"` | — | pc 4, locals `[42, undefined]` |
| 5 | `register_wait` `first:approved::4` | `ack` | `suspended`, pending event wait |
| 6 | `persist_checkpoint` | `persist_confirmed` `{ revision: 2 }` | `suspended`, `revision` 2 |
| 7 | (suspended) | `event_payload` `"ok"` | stack `["ok"]`, pending none |
| 8 | `persist_checkpoint` | `persist_confirmed` `{ revision: 3 }` | `runnable`, `revision` 3 |
| 9 | store approval, build object, `Return`, persist | `persist_confirmed` `{ revision: 4 }` | `completed`, result `{ result: 42, approval: "ok" }` |

The TypeScript frontend emits this artifact. Artifact identity is a hash of the engine program, not of generated JavaScript. Effect callbacks run in the host, not in the artifact.
