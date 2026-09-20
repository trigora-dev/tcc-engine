# Bindings

A binding embeds the TCC core in a host language. It does not compile source and does not persist checkpoints or perform external I/O.

| Path | Role |
|---|---|
| `bindings/javascript` | JavaScript loads `tcc-wasm` through a C ABI over JSON |
| `bindings/python` | Native PyO3 module (`tcc_engine`) over `tcc-core` |

WASM is the JavaScript delivery target and a conformance target for every artifact. Python-hosted execution uses the native binding, not WASM.

## JavaScript

The JavaScript binding loads the WASM module and presents `HostRequest` / `HostResponse` as JSON through linear memory. The WASM crate exports a C ABI (`tcc_start`, `tcc_resume`, `tcc_run_until_host`, `tcc_apply_response`) and does not use WASI.

Build the module with `cargo build -p tcc-wasm --target wasm32-unknown-unknown --release`. The packed `@tcc-engine/bindings-javascript` tarball includes `tcc_wasm.wasm`. `loadEngine()` with no arguments loads that file from disk (Node). Embedders can pass bytes or a `WebAssembly.Module` instead, which is the Cloudflare Workers path.

## Python

The Python binding exposes `EngineBinding` (`start` via the constructor, plus `resume`, `run_until_host`, `apply_response`, `continuation_json`) using the same JSON host protocol. The `tcc-engine` wheel also includes `compile` / `CompileError` and a SQLite host driver. Build it with `maturin develop` from `bindings/python`. This is not the public `trigora` SDK.
