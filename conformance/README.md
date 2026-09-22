# Host conformance v1

Named kit for hosts that speak TCC host protocol version 1. Spec: [`spec/host-conformance-v1.md`](../spec/host-conformance-v1.md). Case index: [`cases.json`](cases.json).

Ordinary-computation compatibility is a separate frozen suite: [`ordinary/`](ordinary/).

The runner asserts observable TCC semantics. A driver operates a specific host. Reconstruct goldens live in [`spec/fixtures/persist/`](../spec/fixtures/persist/). Semantic sources live under [`cases/`](cases/).

## Run

Reference adapters (CI aliases):

```text
node --experimental-sqlite --experimental-strip-types conformance/run-node.ts
python conformance/run_python.py
```

Generic Node runner with a driver module that default-exports `createDriver()`:

```text
node --experimental-sqlite --experimental-strip-types conformance/run.ts --driver ./my-driver.ts
```

Python executor over the same JSON:

```text
python conformance/run.py
```

## Layout

```text
conformance/
  cases.json              # shared corpus
  runner.ts               # Node executor
  run.py                  # Python executor over the same JSON
  drivers/
    node-sqlite.ts
    python_sqlite.py
  run-node.ts             # CI: runner + node-sqlite
  run_python.py           # CI: run.py + python-sqlite
```

## Driver

| Language | Adapter |
|---|---|
| TypeScript | [`drivers/node-sqlite.ts`](drivers/node-sqlite.ts) |
| Python | [`drivers/python_sqlite.py`](drivers/python_sqlite.py) |

`HostConformanceDriver` is semantic only: `applyDelta`, `start`, `crashAt`, `resume`, `readContinuation`, `effectLog`. No SQLite schema, `exec_head`, WAL, or packing knobs.

Crash helpers used by the language suite and by these adapters stay in `hosts/node/src/conformance.ts` and `hosts/python/conformance.py`.
