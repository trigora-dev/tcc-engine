# JavaScript bindings

Loads the WASM module and presents `HostRequest` / `HostResponse` as JSON through linear memory. The WASM crate exports a C ABI (`tcc_start`, `tcc_run_until_host`, `tcc_apply_response`) without WASI.

It does not compile source (that is a frontend) and does not persist checkpoints or perform external I/O (that is a host).

Build the module with `cargo build -p tcc-wasm --target wasm32-unknown-unknown --release`.
