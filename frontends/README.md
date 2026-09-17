# Frontends

A frontend compiles a source language into a TCC program artifact. It is not a binding and not a host.

| Piece | Job |
|---|---|
| Frontend | Compile programs in this language into a resumable artifact |
| Binding | Embed and control the engine from a host language |
| Host | Persistence, effects, events, scheduling |

A Python frontend and a Python binding are different components. This tree currently has a TypeScript frontend at `frontends/typescript`. Further languages belong in `frontends/<language>/`.

## Artifact

Every frontend emits the same versioned envelope (see [spec/program-format.md](../spec/program-format.md)):

- Executable TCC instructions
- Entry points and function metadata
- Source locations for diagnostics
- Required engine features and host capabilities
- Frontend identity, version, and language-semantics version
- Runtime modules, if any

Unknown required features are rejected by `tcc_ir::validate`.

## Language behavior

Sharing an instruction format does not make languages identical. JavaScript numbers, Python integers, exception handling, object identity, and truthiness are not interchangeable. A frontend either lowers those differences into explicit instructions or runtime helpers, or it refuses to compile them.

The TypeScript frontend implements `ts.subset.v1`. A frontend that needs different numeric types, truthiness, or object identity declares new engine features rather than overloading TypeScript’s.

## Libraries

Arbitrary packages do not run inside the Rust/WASM engine. Supported libraries need compilation or runtime support. External operations need an explicit **host capability** (effects, HTTP, timers). Unsupported constructs are compile errors.

## Adding a frontend

1. Place tooling and dependencies under `frontends/<language>/`.
2. Emit the envelope `tcc-ir` already validates.
3. Declare required features for any new instructions.
4. Add conformance tests for the supported subset.
5. Depend on the artifact contract, not on `tcc-core` internals or a particular host.

The TypeScript frontend lives in this repository so compiler, engine, and compatibility changes land together. Independent frontends can follow the published specs.
