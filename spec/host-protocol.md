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

Requests are coarse. A persist of wait registration and continuation suspend must be atomic, or must follow an order from which the host can recover. A committed wait must have a durable wakeup registration.

## Responses

| Response | Meaning |
|---|---|
| `ack` | Request completed with no payload |
| `persist_confirmed` | Checkpoint committed at `revision` |
| `effect_result` | Effect succeeded |
| `effect_failed` | Effect failed |
| `event_payload` | Matching event delivered |
| `child_result` | Child execution completed |
| `artifact` | Requested artifact identity |

A response that does not match the outstanding request is an error.

## Ownership

Exactly one worker may advance a given execution at a time. Hosts must enforce this with transactions, revision checks, leases, or an equivalent mechanism.

## WebAssembly

The WASM module exports this protocol. It does not provide WASI networking, filesystem access, or threads. Those capabilities are supplied by the host.
