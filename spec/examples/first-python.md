# First example (Python)

A Python program that imports durable operations from `trigora` and defines a top-level `async def run()`. There is no context object or workflow wrapper.

The Python frontend treats `trigora` as the durable-operations module. That package is not published from this repository. The frontend recognizes resolved imports of `effect` and `wait_for_event`. Aliasing works:

```python
from trigora import effect as durable_effect
```

A locally declared `def effect` is not durable.

```python
from trigora import effect, wait_for_event

async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
```

The effect callback body is not compiled into the artifact. The host supplies the result for key `generate`. Keys and event names must be string literals. Unsupported constructs are compile errors.

Language semantics: `py.subset.v1`. Required engine features: `ts.control_flow`, `durable.effect`, `durable.wait_for_event`. Required host capabilities: `host.persist_checkpoint`, `host.effect`, `host.event`. The `ts.control_flow` feature names the existing instruction set; it is not TypeScript-specific execution.

Walkthrough execution id: `first`. Locals: `result` = 0, `approval` = 1.

## Instructions

| pc | Instruction |
|---|---|
| 0 | `LoadConst "generate"` |
| 1 | `Effect` |
| 2 | `StoreLocal 0` |
| 3 | `LoadConst "approved"` |
| 4 | `WaitForEvent` |
| 5 | `StoreLocal 1` |
| 6 | `NewObject` |
| 7 | `LoadLocal 0` |
| 8 | `SetProp "result"` |
| 9 | `LoadLocal 1` |
| 10 | `SetProp "approval"` |
| 11 | `Return` |

Host sequence, identities, and completed result match the TypeScript [first example](first.md). Effect idempotency key: `first:generate`. Wait identity: `first:approved::4`. With effect `generate` = `42` and event payload `"ok"`, the result is `{ result: 42, approval: "ok" }`.
