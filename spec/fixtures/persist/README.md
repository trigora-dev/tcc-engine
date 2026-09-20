# Persist reconstruct fixtures

Shared golden vectors for semantic continuation deltas. Cloud hosts (`trigora-platform` Durable Object SQLite) must apply `delta` to `base` and obtain a continuation identical to `expected` (same logical fields as `tcc_state::encode_continuation`).

These files are the handoff. This repository does not contain Durable Object code.

| File | What it covers |
|---|---|
| `store-local.json` | Frame pc + one local slot + operand stack replace |
| `pending-wait.json` | Pending wait + status + empty stack |
| `clear-pending.json` | Pending cleared (`null`) + stack + status |

Reconstruction is host packing. It is not execution of completed instructions. Resume still consumes a full continuation in `spec/continuation-format.md`.
