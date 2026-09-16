# Bindings

A binding exposes the engine to a host language. It loads the native library or the WASM module and presents `HostRequest` / `HostResponse`.

It does not compile source (that is a frontend) and does not persist checkpoints or perform external I/O (that is a host).

The WASM module does not provide WASI networking, filesystem access, or threads. Those capabilities come from the host.
