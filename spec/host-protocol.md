# Host protocol

The engine emits host requests and applies host responses. Bindings present this protocol to a language. Hosts implement it.

Every encoded host request, outcome, and response JSON object must include `host_protocol_version`. The current version is `1`. Absence is a malformed message (`MissingHostProtocolVersion`), not an implicit v1. A present value other than `1` is `UnsupportedHostProtocol`. Bindings stamp `1` when wrapping host objects that omit it; the WASM C ABI and `decode_response` do not.

The engine does not depend on a particular database, operating system, or cloud provider.

Host conformance for this protocol is the named kit in [host-conformance-v1.md](host-conformance-v1.md).

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
| `create_child` | Create the child execution named by `program_name`. Wake the parent when the child is terminal |
| `fetch_artifact` | Return the artifact for the given hash |

`persist_checkpoint` carries persist intent. Naive hosts may ignore everything except `revision` and persist `continuationJson()` as a full snapshot. Optimized hosts (`host.persist_checkpoint_delta`) persist the payload below.

```text
persist_checkpoint
  revision
  kind: snapshot | delta
  base_revision   # last persist_confirmed; 0 if none. Required for delta.
  materialize     # engine intent: host must store a snapshot this commit
  delta?          # semantic ops; null/omitted on snapshot
```

Delta ops name changed continuation fields (frame locals by slot, pc, stack, pending, status, result, try_stack). They are not a byte-diff of JSON. Reconstruction must yield a continuation in `spec/continuation-format.md`. Shared golden vectors live in `spec/fixtures/persist/`.

Hosts persist the snapshot or delta the core emits. They do not drop locals, invent delete ops, or guess liveness. Dead slots are already `undefined` in that payload.

The engine tracks dirtiness against the last **confirmed** revision. A crash before `persist_confirmed` does not advance that base. `engine_format_version` is unchanged; resume still consumes a full continuation.

A host may commit each persist individually or coordinate several transition records into one durability boundary. Coordinated commit is host policy, not a TCC semantic requirement. `persist_confirmed` is valid only after that revision is durable. `ack` still does not commit.

Related records of one logical transition must become durable consistently even on a host that never groups unrelated executions. For `invoke`, that unit is the parent wait checkpoint, the engine-stable child identity, the child execution row, and the parent–child edge. `create_child` `ack` is not a commit. Retry before that commit finds no child; retry after it finds the same child.

The Node and Python reference hosts use a durability coordinator by default. The batch drivers pause each execution with one outstanding `persist_checkpoint`, commit the pending transition records together, then confirm. Newly created children become runnable only after that commit. A failed transaction confirms none; a crash before commit restores each execution from its previous confirmed revision. Coordinated waves measure this path rather than independently advancing executions past uncommitted checkpoints.

Artifacts that list `host.persist_checkpoint` remain valid on optimized hosts. `host.persist_checkpoint_delta` is an additional capability, not a replacement.

Requests are coarse. A persist of wait registration and continuation suspend must be atomic, or must follow an order from which the host can recover (for example an idempotent upsert of `wait_id` so a crash between `register_wait` and `persist_checkpoint` can re-register). A committed wait must have a durable wakeup registration.

## Recovery

The host owns durable state. The engine’s in-memory `outstanding` request is not recoverable.

- Reply `persist_confirmed` only after the continuation (and any wait or effect rows in the same commit) is durable. For optimized hosts that means the WAL record and per-execution head are in a committed group. `ack` on `persist_checkpoint` does not commit.
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
