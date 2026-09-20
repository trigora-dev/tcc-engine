# Hosts

A host supplies persistence, effects, events, and scheduling. It is not a frontend and not a binding.

| Path | Role |
|---|---|
| `hosts/node-memory` | In-memory Node driver with a fake effect provider |
| `hosts/node` | Node driver with a SQLite store |
| `hosts/python` | Python driver with a SQLite store and native PyO3 embedding |

`hosts/node-memory` keeps execution state in process memory and does not survive restart.

`hosts/node` and `hosts/python` commit artifacts, continuations, effect journal rows, and event waits to SQLite. The packed libraries take an injected effect callback (`runEffect` / `run_effect`). Recovery is a new process that opens the same database, loads the artifact named by the continuation, and resumes. The host does not decide program counters or locals. Python-produced artifacts also run through the Node/WASM host as a conformance target. SIGKILL crash hooks stay in the test harness (`TCC_CRASH_AT`).

Default persistence is **optimized**: a shared WAL of snapshot and semantic-delta records, a durability coordinator, and a per-execution head index (`exec_head(execution_id) → { snapshot_rev, wal_seq }`). Coordinated commit is host architecture, not a TCC semantic requirement; a host that commits each checkpoint individually is still valid. The reference Node and Python hosts coordinate by default: they pause one checkpoint per in-flight execution, commit the pending transition records together (including parent wait + child create + parent–child edge for `invoke`), then send `persist_confirmed`. A failed commit confirms none. Newly created children become runnable only after that commit. Recovery loads the last snapshot for that execution plus a suffix of at most 32 deltas. It does not scan WAL records of other executions. Cloud hosts implement the same reconstruct class against Durable Object SQLite; this repository supplies the persist payload and `spec/fixtures/persist/` goldens, not Durable Object code.

Naive overwrite of a full continuation row remains available behind the internal `TCC_PERSIST=naive` flag for A/B benches only. It is not a product API.

Reference Node and Python hosts accept an optional `onEvent` callback (`checkpoint.persisted`, `checkpoint.materialized`, `continuation.restored`, `effect.journal_hit`, `batch.committed`, `child.created`, `child.completed`, `runtime.error`). Fields are host-agnostic (`executionId`, `revision`, `durationMs`, `bytes`, `kind`, `batchSize`, `engineVersion`). The sink must not throw into persist/confirm; program contents are omitted. This is not an OTEL/Datadog exporter and does not include Cloud tenant or worker dimensions.

**Public benchmarks** (Node canonical): see [`bench/README.md`](../bench/README.md) — `pnpm bench`, `pnpm bench:persistence`, `pnpm bench:recovery`. Python mirror: `python bench/python/run.py`.

**Internal engineering harnesses** (not publishable claims): `pnpm --filter @tcc-engine/host-node bench:persist:internal` and `python hosts/python/bench_persist.py`.
