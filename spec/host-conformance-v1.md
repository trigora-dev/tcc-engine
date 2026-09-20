# Host conformance v1

A named kit a host can claim. This repository’s Node and Python SQLite hosts are the reference drivers.

`engine_format_version` stays `1`. Every host-protocol JSON message must declare `host_protocol_version: 1`. Missing is not version 1.

## Claim

A host that claims **host conformance v1** must:

1. Speak `host_protocol_version` `1` on every request and response JSON. Reject missing and unknown versions.
2. Resume only against the stored `artifact.hash` blob (`ArtifactMismatch` on a different compiled artifact or substituted source).
3. Reconstruct golden vectors in [`spec/fixtures/persist/`](fixtures/persist/) by applying `delta` to `base`. Reconstruction is host packing, not SQLite.
4. Recover from SIGKILL at the crash hooks below with the same terminal result and effect log as an uninterrupted run.
5. Treat `persist_confirmed` as the durability boundary. `ack` on `persist_checkpoint` does not commit.

Reconstruct cases are host-agnostic: any `applyDelta` implementation can pass them. Semantic and crash cases require a driver.

## Driver contract

A driver exposes:

| Operation | Meaning |
|---|---|
| `start` | Create an execution from an artifact and run until terminal or suspend |
| `crashAt` | Start in a child process and `SIGKILL` at a named hook (`TCC_CRASH_AT`) |
| `resume` | Open the same durable store and continue from the last committed continuation |
| `readContinuation` | Load the committed continuation JSON for an execution |
| `effectLog` | Ordered effect-provider invocations |

Node wraps [`hosts/node/src/conformance.ts`](../hosts/node/src/conformance.ts). Python wraps [`hosts/python/conformance.py`](../hosts/python/conformance.py). CI still runs those suites; this kit names them.

Kit entry points: [`conformance/run-node.ts`](../conformance/run-node.ts), [`conformance/run_python.py`](../conformance/run_python.py). Case index: [`conformance/cases.json`](../conformance/cases.json).

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

## Out of scope for this kit version

Closures, broader JavaScript/Python, `Promise.all`, extra persist heuristics, Docker, and a third language.
