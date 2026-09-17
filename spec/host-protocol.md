# Host protocol

The engine emits host requests and applies host responses. Bindings present this protocol to a language. Hosts implement it.

The engine does not depend on a particular database, operating system, or cloud provider.

## Roles

| Role | Responsibility |
|---|---|
| Engine | Execute the artifact, own continuation semantics, emit `HostRequest` |
| Binding | Load the native or WASM engine and expose the protocol |
| Host | Persistence, external effects, timers, events, child dispatch, artifact fetch, execution ownership |

## Requests

| Request | Host action |
|---|---|
| `persist_checkpoint` | Commit continuation state. Reply with `persist_confirmed` (including `revision`) or `ack` |
| `run_effect` | Perform the external operation using the supplied identity |
| `persist_effect` | Persist the effect journal record |
| `register_timer` | Arrange a wake at `resume_at_ms` |
| `register_wait` | Persist the event wait. Wake on a matching event or timeout |
| `create_child` | Create the child execution. Wake the parent when the child is terminal |
| `fetch_artifact` | Return the artifact for the given hash |

Requests are coarse. A persist of wait registration and continuation suspend must be atomic, or must follow an order from which the host can recover (for example an idempotent upsert of `wait_id` so a crash between `register_wait` and `persist_checkpoint` can re-register). A committed wait must have a durable wakeup registration.

## Recovery

The host owns durable state. The engine’s in-memory `outstanding` request is not recoverable.

- Reply `persist_confirmed` only after the continuation (and any wait or effect rows in the same commit) is durable. `ack` on `persist_checkpoint` does not commit.
- A crash during a persistence transaction must not leave an execution looking as if that revision committed.
- Resume loads the latest committed continuation and the artifact named by `artifact.hash`. The host supplies that artifact blob; it must not substitute whatever source is currently compiled.
- Exactly one worker may advance an execution. Enforce with a lease or owner token and a revision compare-and-swap on continuation writes.

## Responses

| Response | Meaning |
|---|---|
| `ack` | Request completed with no payload |
| `persist_confirmed` | Checkpoint committed at `revision` |
| `effect_result` | Effect succeeded |
| `effect_failed` | Effect failed |
| `event_payload` | Matching event delivered. Valid when the continuation is `suspended` on an event wait and no persist request is outstanding |
| `timer_fired` | Matching timer delivered. Valid when the continuation is `suspended` on a timer wait |
| `child_result` | Child execution completed. Valid when the continuation is `suspended` on a child wait |
| `cancel` | Host-delivered cancellation. Takes effect at a durable boundary; does not interrupt an in-flight `run_effect` |
| `artifact` | Requested artifact identity |

A response that does not match the outstanding request is an error.

## Ownership

Exactly one worker may advance a given execution at a time. Hosts must enforce this with transactions, revision checks, leases, or an equivalent mechanism.

## WebAssembly

The WASM module exports this protocol. It does not provide WASI networking, filesystem access, or threads. Those capabilities are supplied by the host.
