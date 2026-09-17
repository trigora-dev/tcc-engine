# Hosts

A host supplies persistence, effects, events, and scheduling. It is not a frontend and not a binding.

| Path | Role |
|---|---|
| `hosts/node-memory` | In-memory Node driver with a fake effect provider |

This host is for Phase 2 equivalence tests. It does not survive process restart. SQLite persistence is Phase 3.
