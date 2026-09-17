# JavaScript bindings

The JavaScript binding loads the WASM module and presents `HostRequest` / `HostResponse` as JSON through linear memory. The WASM crate exports a C ABI (`tcc_start`, `tcc_resume`, `tcc_run_until_host`, `tcc_apply_response`) and does not use WASI.

Bindings do not compile source and do not persist checkpoints or perform external I/O.

Build the module with `cargo build -p tcc-wasm --target wasm32-unknown-unknown --release`.
