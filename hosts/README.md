# Hosts

A host supplies persistence, effects, events, and scheduling. It is not a frontend and not a binding.

| Path | Role |
|---|---|
| `hosts/node-memory` | In-memory Node driver with a fake effect provider |
| `hosts/node` | Node driver with a SQLite store |
| `hosts/python` | Python driver with a SQLite store and native PyO3 embedding |

`hosts/node-memory` keeps execution state in process memory and does not survive restart.

`hosts/node` and `hosts/python` commit artifacts, continuations, effect journal rows, and event waits to SQLite. The packed libraries take an injected effect callback (`runEffect` / `run_effect`). Recovery is a new process that opens the same database, loads the artifact named by the continuation, and resumes. The host does not decide program counters or locals. Python-produced artifacts also run through the Node/WASM host as a conformance target. SIGKILL crash hooks stay in the test harness (`TCC_CRASH_AT`).

Default persistence is **optimized**: a shared WAL of snapshot and semantic-delta records, group commit, and a per-execution head index (`exec_head(execution_id) → { snapshot_rev, wal_seq }`). Recovery loads the last snapshot for that execution plus a suffix of at most 32 deltas. It does not scan WAL records of other executions. Cloud hosts implement the same reconstruct class against Durable Object SQLite; this repository supplies the persist payload and `spec/fixtures/persist/` goldens, not Durable Object code.

Naive overwrite of a full continuation row remains available behind the internal `TCC_PERSIST=naive` flag for A/B benches only. It is not a product API. Internal benches: `pnpm --filter @tcc-engine/host-node bench:persist` and `python hosts/python/bench_persist.py`.
