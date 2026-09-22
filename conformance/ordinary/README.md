# Ordinary computation corpus

Permanent compatibility suite for this promise: ordinary non-I/O code keeps compiling and behaving the same around durable boundaries.

This is not a one-time launch benchmark. The programs are the regression proof. Parser coverage is not a substitute.

## Baseline

The programs listed in `manifest.json` are the frozen baseline (40 source files: paired scenarios in `shared/`, TypeScript-only behavior in `typescript/`, Python-only behavior in `python/`).

- Keep those programs frozen.
- A new language feature adds cases. It does not rewrite a baseline program.
- Do not retune an existing program to make the score look better. Change a file only when the program itself is wrong.

Conceptually, `shared/` is the cross-language baseline, `typescript/` and `python/` are language-specific baseline, and any program with an `effect` or `wait` is the mixed-durable slice. Those roles stay in place. Do not move a baseline file to improve the layout.

External work uses `effect` or `waitForEvent` / `wait_for_event`. Computation is not wrapped in an effect.

## What CI requires

`conformance/ordinary/score.ts` runs every baseline program on the native engine (`tcc-drive`), WASM, and the Python binding.

Every baseline program must pass on all three. A miss fails the run. There is no percentage cushion.

Where a program has a durable boundary, the run snapshots the continuation there, resumes it, and requires the same result. The resumed continuation has a single frame.

A non-I/O line is a non-blank, non-comment line that is not a durable import, not an `await` of `effect` / `waitForEvent` / `wait_for_event` / `sleep` / `invoke`, and not inside an effect callback. The line count is reported. It is not a threshold.

## Run

```text
cargo build -p tcc-wasm --target wasm32-unknown-unknown --release
cargo build -p tcc-core --bin tcc-drive --release
node --experimental-strip-types conformance/ordinary/score.ts
```

The Python step needs the native binding (`maturin develop` from `bindings/python`). `TCC_DRIVE` overrides the native binary path.
