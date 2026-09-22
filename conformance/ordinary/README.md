# Ordinary computation corpus

Permanent compatibility suite for this promise: ordinary non-I/O code keeps compiling and behaving the same around durable boundaries.

This is not a one-time launch benchmark. The programs are the regression proof. Parser coverage is not a substitute.

## Two layers

`manifest.json` has two lists. Ids are unique across both.

- `baseline` is the original launch set: 40 source files, frozen on 2026-09-22. Those program texts stay frozen. Change one only when the program itself is wrong, or to fix the harness.
- `suite` is everything added after that launch set. A new language feature appends cases here. It does not rewrite a baseline program. Do not retune an existing program to make the score look better.

Paths are grouped by language and topic. Paired programs live under `shared/<topic>/`. TypeScript-only programs live under `typescript/<topic>/`. Python-only programs live under `python/<topic>/`. Both layers use those folders.

External work uses `effect` or `waitForEvent` / `wait_for_event`. Computation is not wrapped in an effect.

## What CI requires

`conformance/ordinary/score.ts` runs both lists on the native engine (`tcc-drive`), WASM, and the Python binding.

Every program in both lists must pass on all three. A miss in either list fails the run. There is no percentage cushion. The launch bar is the baseline list. The suite is allowed to grow and must also pass.

Where a program has a durable boundary, the run snapshots the continuation there, resumes it, and requires the same result. The resumed continuation has a single frame.

A non-I/O line is a non-blank, non-comment line that is not a durable import, not an `await` of `effect` / `waitForEvent` / `wait_for_event` / `sleep` / `invoke`, and not inside an effect callback. Each list reports its own line count. The count is not a threshold.

## Layout

```text
conformance/ordinary/
  shared/
    control_flow/ collections/ helpers/ durability/ closures/ callbacks/
  typescript/
    arithmetic/ coercion/ iteration/ collections/ helpers/ control_flow/ callbacks/
  python/
    numeric/ defaults/ collections/ iteration/ control_flow/ helpers/ closures/
```

## Run

```text
cargo build -p tcc-wasm --target wasm32-unknown-unknown --release
cargo build -p tcc-core --bin tcc-drive --release
node --experimental-strip-types conformance/ordinary/score.ts
```

The Python step needs the native binding (`maturin develop` from `bindings/python`). `TCC_DRIVE` overrides the native binary path.
