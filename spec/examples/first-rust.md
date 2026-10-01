# First example (Rust)

A Rust program that imports durable operations from `tcc_rust_prelude` and declares exactly one `pub async fn main`. There is no context object or workflow wrapper.

The Rust frontend treats `tcc_rust_prelude` and `trigora` as durable-operations modules. The frontend recognizes resolved imports of `effect` and `wait_for_event`. Aliasing works:

```rust
use tcc_rust_prelude::effect as durable_effect;
```

A locally declared `fn effect` is not durable.

```rust
use tcc_rust_prelude::{effect, wait_for_event};

struct Outcome {
    result: f64,
    approval: String,
}

pub async fn main() -> Result<Outcome, String> {
    let result: f64 = effect("generate", || 42.0).await?;
    let approval: String = wait_for_event("approved").await?;
    Ok(Outcome { result, approval })
}
```

The effect closure is typechecked and omitted from the artifact. The host supplies the result for key `generate`. Keys and event names must be string literals. Unsupported constructs are compile errors.

`effect` always lowers to an empty object, the key, then `Effect` with `has_input: true`. A closure that captures nothing still sends that empty object. `?` on `Result` tests `$tag`: `Ok` continues with the `$0` payload, and `Err` returns. `Outcome` is an object with `result` and `approval`. The function returns that object wrapped as `Ok`.

Language semantics: `rust.subset.v1`. Required engine features: `ts.control_flow`, `lang.compute`, `durable.effect`, `durable.wait_for_event`. Required host capabilities: `host.persist_checkpoint`, `host.effect`, `host.event`. The `ts.control_flow` feature names the existing instruction set; it is not TypeScript-specific execution.

Walkthrough execution id: `first`. Effect idempotency key: `first:generate`. `WaitForEvent` is at pc 29, so the wait identity is `first:approved::29`. With effect `generate` = `42` and event payload `"ok"`, the completed result is `Ok` carrying `Outcome { result: 42, approval: "ok" }`.
