# Persist reconstruct fixtures

Shared golden vectors for semantic continuation deltas. A host that reconstructs from persist intent must apply `delta` to `base` and obtain a continuation identical to `expected` (same logical fields as `tcc_state::encode_continuation`).

Host conformance v1 reconstruct cases point here; see [`spec/host-conformance-v1.md`](../../host-conformance-v1.md) and [`conformance/cases.json`](../../../conformance/cases.json).

| File | What it covers |
|---|---|
| `store-local.json` | Frame pc + one local slot + operand stack replace |
| `pending-wait.json` | Pending wait + status + empty stack |
| `clear-pending.json` | Pending cleared (`null`) + stack + status |
| `slot-undefined.json` | Frame pc + one local slot patched to `{ t: "undefined" }` (ceased to be live). Reconstruct must not resurrect the previous payload. |

Reconstruction is host packing. It is not execution of completed instructions. Resume still consumes a full continuation in `spec/continuation-format.md`.
