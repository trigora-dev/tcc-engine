# Hosts

A host supplies persistence, effects, events, and scheduling. It is not a frontend and not a binding.

| Path | Role |
|---|---|
| `hosts/node-memory` | In-memory Node driver with a fake effect provider |

This host keeps execution state in process memory and does not survive restart. A persistent host is not in this repository yet.
