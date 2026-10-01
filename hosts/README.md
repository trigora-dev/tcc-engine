# Hosts

A host supplies persistence, effects, events, and scheduling. It is not a frontend and not a binding.

| Path | Role |
|---|---|
| `hosts/node-memory` | In-memory Node host |
| `hosts/node` | Node reference host with a SQLite store |
| `hosts/python` | Python reference host with a SQLite store and native PyO3 embedding |

`hosts/node-memory` keeps execution state in process memory and does not survive restart.

`hosts/node` and `hosts/python` are reference hosts. They commit artifacts, continuations, effect journal rows, and event waits to SQLite. They take an injected effect callback (`runEffect` / `run_effect`). Recovery is a new process that opens the same database, loads the artifact named by the continuation, and resumes. The host does not decide program counters or locals. A host that commits each checkpoint on its own is valid; coordinating several checkpoints in one commit is a host choice. Reply `persist_confirmed` only after the commit succeeds. A failed commit confirms none. Newly created children become runnable only after that commit. Embedding walkthrough: [`docs/embed.md`](../docs/embed.md).

Reference Node and Python hosts accept an optional `onEvent` callback (`checkpoint.persisted`, `checkpoint.materialized`, `continuation.restored`, `effect.journal_hit`, `batch.committed`, `child.created`, `child.completed`, `runtime.error`). Fields are host-agnostic (`executionId`, `revision`, `durationMs`, `bytes`, `kind`, `batchSize`, `engineVersion`). The sink must not throw into persist or confirm. Program contents are omitted.

Benchmarks on the Node reference host: [`bench/README.md`](../bench/README.md). Python mirror: `python bench/python/run.py`.
