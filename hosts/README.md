# Hosts

A host supplies persistence, effects, events, and scheduling. It is not a frontend and not a binding.

| Path | Role |
|---|---|
| `hosts/node-memory` | In-memory Node driver with a fake effect provider |
| `hosts/node` | Node driver with a SQLite store |
| `hosts/python` | Python driver with a SQLite store and native PyO3 embedding |

`hosts/node-memory` keeps execution state in process memory and does not survive restart.

`hosts/node` and `hosts/python` commit artifacts, continuations, effect journal rows, and event waits to SQLite. The packed libraries take an injected effect callback (`runEffect` / `run_effect`). Recovery is a new process that opens the same database, loads the artifact named by the continuation, and resumes. The host does not decide program counters or locals. Python-produced artifacts also run through the Node/WASM host as a conformance target. SIGKILL crash hooks stay in the test harness (`TCC_CRASH_AT`).
