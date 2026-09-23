# Ordinary computation corpus

Permanent compatibility suite for this promise: ordinary non-I/O code keeps compiling and behaving the same around durable boundaries.

This is not a one-time launch benchmark. The programs are the regression proof. Parser coverage is not a substitute.

## One index

`manifest.json` is a single `programs` list. Ids are unique. A new language feature appends a program. It does not rewrite an existing program to make the score look better.

Paths are grouped by language and topic.

- A shared case is one directory: `shared/<topic>/<case>/<case>.ts`, `.py`, and `.rs` when the Rust subset can return the same value.
- TypeScript-only programs live under `typescript/<topic>/`.
- Python-only programs live under `python/<topic>/`.
- Rust-only programs live under `rust/<topic>/`.

External work uses `effect` or `waitForEvent` / `wait_for_event`. Computation is not wrapped in an effect.

## What CI requires

`conformance/ordinary/score.ts` runs every program on the native engine (`tcc-drive`), WASM, and the Python binding.

Every program must pass on all three runtimes. A miss fails the run. There is no percentage cushion.

Where a program has a durable boundary, the run snapshots the continuation there, resumes it, and requires the same result. The resumed continuation has a single frame.

A non-I/O line is a non-blank, non-comment line that is not a durable import, not an `await` of `effect` / `waitForEvent` / `wait_for_event` / `sleep` / `invoke`, and not inside an effect callback. The score reports that count. The count is not a threshold.

## Layout

```text
conformance/ordinary/
  shared/<topic>/<case>/<case>.{ts,py,rs}
  typescript/<topic>/*.ts
  python/<topic>/*.py
  rust/<topic>/*.rs
```

Rust topics: `ownership`, `values`, `structs`, `matching`, `collections`, `iteration`, `helpers`, `closures`, `durability`.

## Run

```text
cargo build -p tcc-wasm --target wasm32-unknown-unknown --release
cargo build -p tcc-core --bin tcc-drive --release
cargo build -p tcc-rust-frontend --release
node --experimental-strip-types conformance/ordinary/score.ts
```

The Python step needs the native binding (`maturin develop` from `bindings/python`). `TCC_DRIVE` overrides the native binary path. `TCC_RUSTC` overrides the Rust compiler path.
