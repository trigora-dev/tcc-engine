# Embed TCC Engine

Compile a supported program, run it on a reference SQLite host with your own effects, kill the process, and resume from the same database. There is no CLI or managed runtime in this path.

The reference host is a library, not a service. It does not provide a control plane, autoscaling, fleet management, or operational guarantees.

## Packages

**Node** (Node 22, `--experimental-sqlite`):

```text
@tcc-engine/primitives
@tcc-engine/frontend-typescript
@tcc-engine/bindings-javascript
@tcc-engine/host-node
```

**Python:** one `tcc-engine` wheel. Author with `tcc_engine.primitives`. Embed with `compile`, `EngineBinding`, `Store`, `start_execution`, `resume_execution`.

Local tarballs and wheels: see the root README. Packs are not on a public registry yet.

## Authoring

Engine-native spelling (canonical for embedders):

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

```python
from tcc_engine.primitives import effect, wait_for_event

async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
```

Trigora spelling (`@trigora/sdk` / `trigora`) is still accepted and lowers to the same artifact. A locally declared `function effect` is not durable. Effect callbacks are not compiled; the host answers by string-literal key.

Supported source is the [TypeScript](../spec/typescript-subset.md) and [Python](../spec/python-subset.md) subsets. Arbitrary libraries do not run inside the engine.

## Node: compile, start, resume

```ts
import { compile } from "@tcc-engine/frontend-typescript";
import { loadEngine } from "@tcc-engine/bindings-javascript";
import { startExecution, resumeExecution } from "@tcc-engine/host-node";

const artifact = compile(source, { filename: "first.ts" });
await loadEngine();

const started = await startExecution({
  dbPath: "./state.db",
  artifactJson: JSON.stringify(artifact),
  runEffect: (key) => {
    if (key === "generate") {
      return 42;
    }
    throw new Error(key);
  },
  autoDeliverEvent: false,
});

const resumed = await resumeExecution({
  dbPath: "./state.db",
  runEffect: (key) => {
    if (key === "generate") {
      return 42;
    }
    throw new Error(key);
  },
  eventPayload: "ok",
  autoDeliverEvent: false,
});
```

`runEffect` is `(key: string) => unknown`. There is no `Host` class.

For a shared database, construct a `Store` and call `runOnStore(store, options)` (or `runBatchOnStore` for coordinated waves). `startExecution` opens `dbPath` and closes it when the drive returns.

`autoDeliverEvent` defaults to `true` as a test convenience: if you pass `eventPayload`, the host may deliver it before returning. For a real integration, set `autoDeliverEvent: false` and deliver events, timers, child results, and cancel from your process by calling `resumeExecution`.

Kill the process after `startExecution` has committed; `resumeExecution` on the same `dbPath` loads the last committed continuation and the artifact stored under `artifact.hash`.

## Python: compile, start, resume

```python
from tcc_engine import compile, start_execution, resume_execution
from tcc_engine.compile import artifact_json

artifact = compile(source, filename="first.py")

def run_effect(key: str):
    if key != "generate":
        raise RuntimeError(key)
    return 42

started = start_execution(
    db_path="./state.db",
    artifact_json=artifact_json(artifact),
    run_effect=run_effect,
    auto_deliver_event=False,
)

resumed = resume_execution(
    db_path="./state.db",
    run_effect=run_effect,
    event_payload="ok",
    auto_deliver_event=False,
)
```

`Store` and `run_on_store` are the shared-database path. Do not import authoring names from `tcc_engine` itself: use `tcc_engine.primitives`.

## Custom hosts

Implement [host protocol v1](../spec/host-protocol.md) against `EngineBinding`. Persist continuations, the effect journal, waits, timers, children, and artifact blobs. Reply `persist_confirmed` only after commit. Resume the stored artifact, not whatever source is currently compiled.

[Host conformance v1](../spec/host-conformance-v1.md) and the reconstruct goldens in [`spec/fixtures/persist/`](../spec/fixtures/persist/) are the correctness target. Semantic and crash cases today wrap the Node and Python reference drivers; a Postgres host would implement the driver contract itself. There is no `createPostgresHost()` helper.

## What this path does not include

No Docker production server, Kubernetes operator, HA cluster, distributed scheduler, or admin UI. Those belong to a managed or commercially licensed production offering, not the reference library.
