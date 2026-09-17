# TCC Engine

A portable compiler and runtime for recovering programs from committed continuation state.

TCC—Transparent Continuation Checkpointing—records the program position and live values needed to continue execution. After a failure, a runtime restores that continuation and resumes without re-executing the completed prefix.

The core is Rust. It builds natively and to WebAssembly. A host supplies persistence, external effects, event delivery, and scheduling.

## How it works

Programs use explicit durable operations. A language frontend compiles supported source into a TCC artifact. The engine executes the artifact and stops at durable boundaries for the host. After the host commits the required state, the execution can be restored from that continuation and the artifact that produced it.

```text
Program source
      │
      ▼
Language frontend
      │
      ▼
TCC program artifact
      │
      ▼
Rust / WASM engine
      │
      ▼
Host
Storage · Effects · Events · Scheduling
```

A host may keep an execution history for inspection. Recovery does not use that history to reconstruct program state.

## Recovery

Recovery loads the latest committed continuation.

- A continuation is valid only with the artifact that produced it. The engine does not migrate executions onto incompatible code.
- Restore cost depends on continuation size and the host's persistence implementation. It is not specified as constant-time.
- An external operation may succeed before its result is committed. Ambiguous outcomes are represented explicitly; idempotency is the provider's and the application's responsibility.
- Supported language constructs are those accepted by a frontend. Arbitrary TypeScript, Python, or third-party libraries are not implied.
- Persistence, wakeup, and fencing guarantees are properties of the host.

Contracts are in [spec/](spec/program-format.md).

## This repository

This repository is the engine, its specifications, compiler frontends, and embeddings.

Application authors should use [Trigora](https://github.com/trigora-dev/trigora) (SDK, client, and CLI). Managed hosting is separate from this engine.

Use this repository to:

- Read the execution model
- Embed the engine
- Implement a host
- Implement a language frontend

## Layout

| Path | Role |
|---|---|
| `crates/tcc-ir` | Program representation and validation |
| `crates/tcc-state` | Values and continuation encoding |
| `crates/tcc-core` | Execution |
| `crates/tcc-host` | Host protocol types and driver |
| `crates/tcc-wasm` | WebAssembly exports |
| `crates/tcc-python` | Native Python (PyO3) embedding |
| `frontends/` | Language frontends |
| `bindings/` | Language embeddings of the engine |
| `hosts/` | Reference hosts |
| `spec/` | Specifications |

A frontend compiles source to a TCC artifact. A binding loads the engine in a host language. They are not the same thing.

The execution core does not depend on a particular database, cloud provider, or SDK. WASM is a delivery and conformance target for that core, not TCC itself. Native embedding is the other path. The WASM target is `wasm32-unknown-unknown` and does not use WASI networking, filesystems, or threads.

## Status

The repository is under development. Present:

- Artifact envelope and validation
- Continuation encoding
- Host protocol
- Stepper, objects, arrays, control flow, exceptions, timers, cancellation, and child invoke
- TypeScript frontend that compiles `@trigora/sdk` durable operations (`effect`, `waitForEvent`, `sleep`, `invoke`)
- Python frontend that compiles `trigora` durable operations (`effect`, `wait_for_event`, `sleep`, `invoke`)
- WASM C ABI, JavaScript binding, and an in-memory Node host
- Native PyO3 binding and a SQLite Python host
- SQLite Node host with process-restart recovery and a conformance helper

Not present: a published SDK.

## Development

Requires a current stable Rust toolchain.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```

```sh
pnpm --dir frontends/typescript install
pnpm --dir frontends/typescript test
cargo build -p tcc-wasm --target wasm32-unknown-unknown --release
node --experimental-strip-types --test hosts/node-memory/src/host.test.ts
node --experimental-sqlite --experimental-strip-types --test hosts/node/src/host.test.ts hosts/node/src/restart.test.ts hosts/node/src/conformance.test.ts
```

```sh
python -m venv .venv
.venv/bin/pip install maturin pytest
.venv/bin/maturin develop --manifest-path crates/tcc-python/Cargo.toml
.venv/bin/pytest frontends/python hosts/python
```

## Specifications

- [Program format](spec/program-format.md)
- [Execution semantics](spec/execution-semantics.md)
- [TypeScript subset](spec/typescript-subset.md)
- [Python subset](spec/python-subset.md)
- [First example](spec/examples/first.md)
- [First example (Python)](spec/examples/first-python.md)
- [Continuation format](spec/continuation-format.md)
- [Host protocol](spec/host-protocol.md)
- [Durable operations](spec/durable-operations.md)
- [Compatibility](spec/compatibility.md)

## Links

- [trigora.dev](https://trigora.dev)
- [SDK and CLI](https://github.com/trigora-dev/trigora)
- [Recovery demo](https://demo.trigora.dev)
