<p align="center">
  <img src="https://trigora.dev/tcc-banner.png" alt="TCC Engine — Portable durable execution without history replay" width="100%" />
</p>

# TCC Engine

**A portable compiler and runtime for durable execution from committed continuation state.**

TCC—Transparent Continuation Checkpointing—records the program position and live values needed to continue execution. After a failure, a runtime restores that continuation and resumes without re-executing the completed prefix.

TCC Engine `26.10.1` is the current public release. `26.10.0` was the first public CalVer cut; `26.10.1` fixes Windows compilation of the SQLite host crash handler.

TCC Engine is the portable execution engine underlying [Trigora](https://trigora.dev). You can embed it directly for development, testing, evaluation, and other uses permitted by the license, or use Trigora for a managed production runtime.

The core is Rust. It builds natively and to WebAssembly. A host supplies persistence, external effects, event delivery, and scheduling.

## Two paths

**Use Trigora** when you want the product surface: language authoring packages, the CLI, and a managed production runtime. Start at [trigora.dev](https://trigora.dev).

**Embed TCC Engine** when you want the compiler and the native or WebAssembly binding in your own process. A host you write uses the published host crates. Guide: [docs/embed.md](docs/embed.md).

The reference host is embeddable infrastructure, not a complete service: your process supplies effects, events, timers, and resume.

## Authoring

Trigora spelling compiles to the same durable operations as the engine-native imports.

| Language | Durable operations |
|---|---|
| [TypeScript](https://github.com/trigora-dev/trigora-typescript) | `@tcc-engine/primitives` or `@trigora/sdk` |
| [Python](https://github.com/trigora-dev/trigora-python) | `tcc_engine.primitives` or `trigora` |
| [Rust](https://github.com/trigora-dev/trigora-rust) | `tcc_rust_prelude` or `trigora` |

## How it works

Programs use explicit durable operations. A language frontend compiles supported source into a TCC artifact. The engine executes the artifact and stops at durable boundaries for the host. After the host commits the required state, the execution can be restored from that continuation and the artifact that produced it.

A host may keep an execution history for inspection. Recovery does not use that history to reconstruct program state.

## Recovery

Recovery loads the latest committed continuation.

- A continuation is valid only with the artifact that produced it. The engine does not migrate executions onto incompatible code.
- Restore cost depends on continuation size and the host's persistence implementation. It is not specified as constant-time.
- An external operation may succeed before its result is committed. Ambiguous outcomes are represented explicitly; idempotency is the provider's and the application's responsibility.
- Supported language constructs are those accepted by a frontend. Arbitrary TypeScript, Python, Rust, or third-party libraries are not implied.
- Persistence, wakeup, and fencing guarantees are properties of the host.

## Host protocol and conformance

A custom host talks [host protocol v1](spec/host-protocol.md) to the binding. It does not need the reference SQLite hosts or Trigora.

[Host conformance v1](spec/host-conformance-v1.md) is a named claim: protocol versioning, artifact pinning, continuation reconstruction, and recovery after process termination at documented crash hooks. The kit is a generic runner plus host drivers. Semantic cases wrap the Node and Python SQLite adapters in this repository; a third-party host implements the same driver contract.

## Install

### TypeScript

```sh
npm install @tcc-engine/frontend-typescript @tcc-engine/bindings-javascript
```

`@tcc-engine/bindings-javascript` loads the engine through WebAssembly.

### Python

```sh
pip install tcc-engine
```

`tcc-engine` includes the Python frontend, `tcc_engine.primitives`, and a SQLite host.

### Rust

```sh
cargo add tcc-ir tcc-state tcc-core tcc-host tcc-host-sqlite tcc-rust-frontend
```

`tcc-rust-frontend` compiles Rust programs. A host you embed uses `tcc-host` and, for SQLite, `tcc-host-sqlite`.

The repository also contains workspace packages used by the TypeScript authoring frontend and Node reference host.

## Repository structure

This repository is the engine, its specifications, compiler frontends, embeddings, reference hosts, and conformance kit. Use it to read the execution model, embed the engine, implement a host, or implement a language frontend.

| Path | Role |
|---|---|
| `primitives/` | TypeScript authoring workspace package |
| `crates/tcc-ir` | Program representation and validation |
| `crates/tcc-state` | Values and continuation encoding |
| `crates/tcc-core` | Execution |
| `crates/tcc-host` | Host protocol types and driver |
| `crates/tcc-host-sqlite` | SQLite host |
| `crates/tcc-wasm` | WebAssembly exports |
| `crates/tcc-python` | Native Python (PyO3) embedding |
| `frontends/` | TypeScript, Python, and Rust frontends |
| `bindings/` | Language embeddings of the engine |
| `hosts/` | Reference hosts |
| `conformance/` | Host conformance v1 kit |
| `spec/` | Specifications |

A frontend compiles source to a TCC artifact. A binding loads the engine in a host language. They are not the same thing.

The execution core does not depend on a particular database, cloud provider, or SDK. WASM is a delivery and conformance target for that core, not TCC itself. Native embedding is the other path. The WASM target is `wasm32-unknown-unknown` and does not use WASI networking, filesystems, or threads.

## Development

Requires a current stable Rust toolchain.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```

```sh
pnpm install
pnpm --filter @tcc-engine/frontend-typescript test
cargo build -p tcc-wasm --target wasm32-unknown-unknown --release
node --experimental-strip-types --test hosts/node-memory/src/host.test.ts
pnpm --filter @tcc-engine/host-node test
```

```sh
python -m venv .venv
.venv/bin/pip install maturin pytest
( cd bindings/python && ../../.venv/bin/maturin develop )
.venv/bin/pytest frontends/python hosts/python
```

## Specifications

- [Host protocol](spec/host-protocol.md)
- [Host conformance v1](spec/host-conformance-v1.md)
- [Durable operations](spec/durable-operations.md)
- [Program format](spec/program-format.md)
- [Execution semantics](spec/execution-semantics.md)
- [TypeScript subset](spec/typescript-subset.md)
- [Python subset](spec/python-subset.md)
- [Rust subset](spec/rust-subset.md)
- [First example](spec/examples/first.md)
- [First example (Python)](spec/examples/first-python.md)
- [First example (Rust)](spec/examples/first-rust.md)
- [Continuation format](spec/continuation-format.md)
- [Compatibility](spec/compatibility.md)

## Links

- **Website:** [trigora.dev](https://trigora.dev)
- **Cloud:** [cloud.trigora.dev](https://cloud.trigora.dev)
- **Docs:** [trigora.dev/docs](https://trigora.dev/docs)
- **Research:** [trigora.dev/research](https://trigora.dev/research)
- **GitHub:** [github.com/trigora-dev/trigora](https://github.com/trigora-dev/trigora)

## License

TCC Engine is licensed under the **Business Source License 1.1 (BUSL-1.1)**.

Production use is permitted under the Additional Use Grant subject to the license terms. Competitive hosted or embedded execution offerings may require a commercial license.

Each release converts to **Apache License 2.0** on its Change Date.

See [LICENSE](LICENSE) for the full terms.

For commercial licensing: [licensing@trigora.dev](mailto:licensing@trigora.dev)