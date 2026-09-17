# Hosts

A host supplies persistence, effects, events, and scheduling. It is not a frontend and not a binding.

| Path | Role |
|---|---|
| `hosts/node-memory` | In-memory Node driver with a fake effect provider |
| `hosts/node` | Node driver with a SQLite store |

`hosts/node-memory` keeps execution state in process memory and does not survive restart.

`hosts/node` commits artifacts, continuations, effect journal rows, and event waits to SQLite. Recovery is a new process that opens the same database, loads the artifact named by the continuation, and resumes. The host does not decide program counters or locals.
