# Bindings

A binding embeds the TCC core in a host language. It does not compile source and does not persist checkpoints or perform external I/O.

| Path | Role |
|---|---|
| `bindings/javascript` | JavaScript loads `tcc-wasm` through a C ABI over JSON |
| `bindings/python` | Native PyO3 module (`tcc_engine`) over `tcc-core` |

WASM is the JavaScript delivery target and a conformance target for every artifact. Python-hosted execution uses the native binding, not WASM.

## JavaScript

The JavaScript binding loads the WASM module and presents `HostRequest` / `HostResponse` as JSON through linear memory. The WASM crate exports a C ABI (`tcc_start`, `tcc_resume`, `tcc_run_until_host`, `tcc_apply_response`) and does not use WASI.

Build the module with `cargo build -p tcc-wasm --target wasm32-unknown-unknown --release`.

## Python

The Python binding exposes `EngineBinding` (`start` via the constructor, plus `resume`, `run_until_host`, `apply_response`, `continuation_json`) using the same JSON host protocol. Build it with `maturin develop --manifest-path crates/tcc-python/Cargo.toml`. This is not the public `trigora` SDK.
