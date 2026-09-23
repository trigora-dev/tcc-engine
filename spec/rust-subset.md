# Rust subset

`language_semantics_version` `rust.subset.v1`. Rust is a launch language beside TypeScript and Python. A construct is supported only when it compiles, runs natively and through WASM, and recovers from a single-frame resume at every durable boundary with the same observable result.

`engine_format_version` stays 1. This frontend adds no opcodes, heap tags, or durability rules.

Authoring imports durable operations from `tcc_rust_prelude`. The prelude exists so `cargo check` can typecheck the source. The lowerer recognizes the same call syntax and does not execute the prelude bodies.

## Move rule

Compound values (`String`, structs, enums, `Vec`) move in the source. The continuation stores them as heap refs. A move stores the destination and then stores `undefined` into the source. The lowerer emits no later read of that source. A VM dump that still shows a ref in the destination is not source-level aliasing.

`f64` and `bool` copy.

`cargo check` against the prelude owns move checking, types, and `match` exhaustiveness. The lowerer is a separate `syn` walk. It does not call rustc and it does not resolve types that are not visible in the syntax. A value whose type is not written at the binding needs a type ascription (`let approved: bool = ...`). A construct that needs HIR or MIR is rejected.

Frontend tests run `cargo check` before `compile`. A fixture `cargo check` rejects is not passed to the lowerer.

## Supported

- `async fn run` with plain owned parameters. Argument binding is exact arity with no defaults
- Ordinary `fn` helpers. They are `Call`. They cannot await. A durable operation runs only while the entry frame is alone
- `let` / `let mut`, assignment, and early `return`
- `if` / `else`, `loop`, `while`, `for x in xs` over `Vec<f64>`, and `for i in 0..n` as a counted `f64` loop. `i32` and every other integer type written in the source are rejected. `as f64` is a no-op on that loop binding
- One struct of named fields and one enum per file. Structs are objects keyed by field names. Enums, `Option`, and `Result` are objects with compiler-only `$tag` and `$0` keys. A struct variant stores its field names beside `$tag`
- One-level struct, enum, and `Option` patterns. No guards, or-patterns, or rest. Nested patterns such as `Some(Job { count })` are a later gap
- `?` on a subset `Result`: a tag test plus `return Err(e)`. It does not emit `Throw`
- `Vec` push, len, and index. `String` is an owned value with no mutation method
- `move` closures for ordinary computation, through the existing cell, environment, and `CallClosure` instructions. Capturing by reference is rejected. The `move` keyword is required when a closure captures
- Durable intrinsics, entry frame only. Names are string literals (`"approved"`). The prelude types those parameters as `&'static str`. That is not general borrow support: `&` and `&mut` anywhere else, including `&T` and `&mut T` in user code, are rejected. An effect closure is typechecked and omitted from the artifact. The host supplies the result. `wait_for_event` and `invoke` take their result type from the ascription

## Operators

```text
f64:    +  -  *  /  %    <  <=  >  >=  ==  !=
bool:   &&  ||  !  ==  !=
String: ==  !=
```

No `**`. No `String` concatenation. Structs and enums are compared only by `match`.

## Durable calls

`invoke("child", (a, b, c)).await` is the v1 spelling. The call takes one owned tuple because Rust has no varargs and a macro is outside this subset. A bare value is a one-element vector (`invoke("child", a)`). `()` is an empty vector. The lowerer flattens that tuple into the existing `Invoke { arg_count }` vector. Rust does not get a one-argument engine special case.

`join(a, b).await` and `race(a, b).await` are two-argument compiler intrinsics. The arguments are direct durable calls. They are not stored. They lower to `Fork` plus `JoinAll` or `JoinAny`. `join` yields `Vec<T>` in input order. `race` yields the winning `T`. Both branches share `T`. Exactly two is a frontend spelling limit. The engine still allows 1 through 32 branches. See [concurrency.md](concurrency.md).

## Rejected

With a span diagnostic: `&` / `&mut` outside the durable-name literal, lifetimes, user generics, traits, `impl`, macros, derives, iterator adapters, async helpers, generators, threads, `unsafe`, `Drop`, integer types, and imports other than the prelude.
