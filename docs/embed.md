# Embed TCC Engine

Compile a supported program, run it with persistent storage, stop the process, and resume from the committed continuation.

A host you write is the integration path. It talks to the published binding or to `tcc-host`. The Node SQLite host in this repository is a reference host for local recovery. It is not the production integration path.

## Install

```sh
npm install @tcc-engine/frontend-typescript @tcc-engine/bindings-javascript
pip install tcc-engine
```

From crates.io: `tcc-host`, `tcc-host-sqlite`, `tcc-rust-frontend`, and the engine crates `tcc-ir`, `tcc-state`, and `tcc-core`.

The `tcc-engine` wheel includes `compile`, `tcc_engine.primitives`, and a SQLite host. `tcc-rust-frontend` compiles Rust programs. `@tcc-engine/bindings-javascript` loads the engine through WebAssembly.

The repository also contains workspace packages used by the TypeScript authoring frontend and Node reference host.

## Authoring

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
from tcc_engine.primitives import program, effect, wait_for_event

@program
async def research():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
```

```rust
use tcc_rust_prelude::{effect, wait_for_event};

pub async fn main() -> Result<f64, String> {
    let result: f64 = effect("generate", || 42.0).await?;
    let _approval: String = wait_for_event("approved").await?;
    Ok(result)
}
```

Trigora spelling (`@trigora/sdk` / `trigora`) lowers to the same artifact. A locally declared `function effect` is not durable. Effect callbacks are not compiled; the host answers by string-literal key.

Supported source is the [TypeScript](../spec/typescript-subset.md), [Python](../spec/python-subset.md), and [Rust](../spec/rust-subset.md) subsets. Arbitrary libraries do not run inside the engine.

## Python

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

`Store` and `run_on_store` are the shared-database path. Import authoring names from `tcc_engine.primitives`. Embedding APIs (`compile`, `Store`, `EngineBinding`) stay on `tcc_engine`.

If you pass `event_payload` and leave `auto_deliver_event` at its default (`true`), the host may deliver that event before returning. Set `auto_deliver_event` to `false` and deliver events, timers, child results, and cancellation from your process by calling `resume_execution`.

Stop the process after `start_execution` has committed. `resume_execution` on the same database loads the last committed continuation and the artifact stored under `artifact.hash`.

## Custom hosts

Implement [host protocol v1](../spec/host-protocol.md) against `EngineBinding`, or against `tcc-host` and `tcc-host-sqlite`. Persist continuations, the effect journal, waits, timers, children, and artifact blobs. Reply `persist_confirmed` only after commit. Resume the stored artifact, not whatever source is currently compiled.

[Host conformance v1](../spec/host-conformance-v1.md) and the reconstruct goldens in [`spec/fixtures/persist/`](../spec/fixtures/persist/) are the correctness target. Implement `HostConformanceDriver` (`applyDelta`, `start`, `crashAt`, `resume`, `readContinuation`, `effectLog`) and run [`conformance/runner.ts`](../conformance/runner.ts) with `--driver`. This repository includes Node and Python SQLite reference hosts. A host you write implements the same contract.

## Node reference host

The Node SQLite host in `hosts/node` is a reference host in this repository. Use it to exercise compile, kill, and resume locally. A production integration implements the host protocol against `@tcc-engine/bindings-javascript` or `tcc-host`.

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

`runEffect` is `(key: string) => unknown`.

For a shared database, construct a `Store` and call `runOnStore(store, options)` (or `runBatchOnStore` for coordinated waves). `startExecution` opens `dbPath` and closes it when the drive returns.

If you pass `eventPayload` and leave `autoDeliverEvent` at its default (`true`), the reference host may deliver that event before returning. Set `autoDeliverEvent` to `false` and deliver events, timers, child results, and cancellation from your process by calling `resumeExecution`.

Stop the process after `startExecution` has committed. `resumeExecution` on the same `dbPath` loads the last committed continuation and the artifact stored under `artifact.hash`. The reference host needs Node 22 and `--experimental-sqlite`.

## What this path does not include

This library does not include a production server, a cluster scheduler, or an admin UI. Those belong to a managed runtime. The reference host does not provide a control plane, autoscaling, or fleet management.
