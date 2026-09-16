# Frontends

A **frontend** translates a source language into a TCC program artifact. It is not a binding and not a host.

| Piece | Job |
|---|---|
| Frontend | Make programs in this language resumable |
| Binding | Let an application in some language embed and control the engine |
| Host | Persistence, effects, events, scheduling |

A Python frontend and a Python binding are different work. Do not add empty language directories. Add `frontends/<language>/` when that frontend exists.

## Artifact

Every frontend emits the same versioned envelope (see [spec/program-format.md](../spec/program-format.md)):

- Executable TCC instructions
- Entry points and function metadata
- Source locations for diagnostics
- Required engine features and host capabilities
- Frontend identity, version, and language-semantics version
- Runtime modules, if any

`tcc_ir::validate` rejects unknown required features. The engine does not guess.

## Language behavior

Sharing an instruction format does not make languages identical. JavaScript numbers, Python integers, exception handling, object identity, and truthiness must not silently collapse. Lower differences into explicit instructions or runtime helpers, or fail compilation.

The first IR is the TypeScript subset actually implemented here (`ts.subset.v1`). Extend with new feature ids when another frontend needs them.

## Libraries

Arbitrary packages do not run inside the Rust/WASM engine. Supported libraries need compilation or runtime support. External operations need an explicit **host capability** (effects, HTTP, timers). Keep that boundary in the frontend: unsupported constructs are compile errors, not weakened recovery.

## Adding a frontend

1. Own tooling and dependencies under `frontends/<language>/`.
2. Emit the envelope `tcc-ir` already validates.
3. Declare required features for any new instructions.
4. Add conformance cases when the tests directory exists.
5. Do not depend on `tcc-core` internals or a particular host.

Once the contract is stable, independent frontend repos can depend on the published specs and conformance tests. First-party frontends stay here so compiler, engine, and compatibility changes land together.
