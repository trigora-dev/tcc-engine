# Host conformance v1

A named kit a host can claim. This repository ships a generic runner plus Node and Python SQLite adapters. Trigora Cloud is not a driver in this repository.

`engine_format_version` stays `1`. Every host-protocol JSON message must declare `host_protocol_version: 1`. Missing is not version 1.

## Claim

A host that claims **host conformance v1** must:

1. Speak `host_protocol_version` `1` on every request and response JSON. Reject missing and unknown versions.
2. Resume only against the stored `artifact.hash` blob (`ArtifactMismatch` on a different compiled artifact or substituted source).
3. Reconstruct golden vectors in [`spec/fixtures/persist/`](fixtures/persist/) by applying `delta` to `base`. Reconstruction is host packing, not SQLite.
4. Recover from SIGKILL at the crash hooks below with the same terminal result and effect log as an uninterrupted run.
5. Treat `persist_confirmed` as the durability boundary. `ack` on `persist_checkpoint` does not commit.

The claim is semantic. Passing does not require SQLite, `exec_head`, a WAL schema, group-commit policy, or a packing heuristic.

## Driver contract

The runner asserts observable TCC semantics. A driver operates one host:

```ts
export type HostConformanceDriver = {
  applyDelta(base: ContinuationJson, delta: ContinuationDelta): ContinuationJson;
  start(input: StartInput): Promise<RunResult>;
  crashAt(input: CrashInput): Promise<Handle>;
  resume(input: ResumeInput): Promise<RunResult>;
  readContinuation(handle: Handle): Promise<ContinuationJson>;
  effectLog(handle: Handle): Promise<string[]>;
};
```

| Operation | Meaning |
|---|---|
| `applyDelta` | The host’s reconstruct function. Goldens are continuation JSON, not SQLite. Snapshot-only hosts may ship the kit’s reference `applyDelta` in [`conformance/reconstruct.ts`](../conformance/reconstruct.ts) / [`conformance/reconstruct.py`](../conformance/reconstruct.py). |
| `start` | Create an execution from an artifact and run until terminal or suspend. |
| `crashAt` | Start an execution, kill the worker at a named hook, and return an opaque handle to the durable workspace. |
| `resume` | Continue from the last committed continuation. A different compiled artifact must fail with an artifact-mismatch error. |
| `readContinuation` | Load the committed continuation JSON for an execution. |
| `effectLog` | Ordered effect-provider invocations. |

Third parties implement this interface against their own host. This repository’s runner also requires `language` and `compile` so it can load the shared TypeScript/Python sources. Adapters are [`conformance/drivers/node-sqlite.ts`](../conformance/drivers/node-sqlite.ts) and [`conformance/drivers/python_sqlite.py`](../conformance/drivers/python_sqlite.py). Reference-host crash helpers stay in [`hosts/node/src/conformance.ts`](../hosts/node/src/conformance.ts) and [`hosts/python/conformance.py`](../hosts/python/conformance.py) for the language suite.

Kit entry points: [`conformance/run-node.ts`](../conformance/run-node.ts), [`conformance/run_python.py`](../conformance/run_python.py). Public runner: [`conformance/runner.ts`](../conformance/runner.ts) (`--driver` via [`conformance/run.ts`](../conformance/run.ts)). Case index: [`conformance/cases.json`](../conformance/cases.json).

## Crash hooks

These names are `TCC_CRASH_AT` values in the reference hosts (`hosts/node/src/host.ts`, `hosts/python` via `tcc_engine.host`). A hook may be qualified as `name:n` or `name:detail`.

| Hook | When |
|---|---|
| `before_persist_checkpoint` | Persist intent received, before the durability commit |
| `after_persist_checkpoint` | After `persist_confirmed` |
| `after_wait_checkpoint` | After the wait-registration persist commits |
| `before_persist_effect` | Before the effect journal row is committed |
| `after_persist_effect` | After the effect journal row is committed (optional `:key`) |
| `before_effect_provider` | Before the effect callback runs |
| `after_effect_provider` | After the effect callback returns |
| `after_register_wait` | After the wait row is upserted |
| `after_register_timer` | After a sleep timer is registered |
| `after_create_child` | After `create_child` is acknowledged (not a commit) |
| `before_event_payload` | Before delivering a matching event |
| `after_event_persist` | After the event delivery persist |
| `before_timer_fired` | Before delivering a timer wake |
| `before_child_result` | Before delivering a child result |
| `before_cancel` | Before host-delivered cancel |

## Artifact pinning

Resume must load the blob stored under `artifact.hash`. Passing a different compiled artifact, or compiling mutated source and substituting it, must fail with `ArtifactMismatch`. The host must not resume against “whatever is currently compiled.”

## Concurrent groups

`concurrent_group.all` and `concurrent_group.any` are part of this kit. `Promise.any` and `Promise.allSettled` are not. Cases live in [`conformance/cases.json`](../conformance/cases.json) and are specified in [concurrency.md](concurrency.md):

- multiple waits, including the same event name
- correlated deliveries (`branch` required; omitted `branch` is rejected)
- reverse completion order, with the aggregate still in input order
- crash after partial completion
- multiple children, no duplicates
- fail-fast plus a detached sibling
- loop re-entry does not alias `branch_id`

`concurrent_group.any` covers `Promise.race`:

- a wait that wins over a timer, and a timer that wins under reverse delivery
- two `waitForEvent` branches with the same name: one event settles branch 0 and detaches branch 1
- crash after the winner checkpoint, including a rejecting race inside `try` / `catch` whose catch body runs once
- an invoke winner whose sibling child was already created and is not cancelled
- loop re-entry does not alias `branch_id`

The TypeScript compiler produces the artifact for these cases. The Python frontend lowers `gather` and `race` to the same joins. The Python SQLite adapter still runs the compiled artifact. Drivers accept `completionOrder: "source" | "reverse"` so one event is delivered to one wait in that order.

## Out of scope for this kit version

Closures, broader JavaScript/Python, `Promise.any`, `Promise.allSettled`, extra persist heuristics, Docker, and a third language. The language recovery suite (`if`/`else`, loops, invoke, cancel, sleep, `Promise.all` and `Promise.race` crash and liveness) stays on the reference hosts in addition to the kit cases above.
