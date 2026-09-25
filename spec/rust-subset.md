# Rust subset

`language_semantics_version` `rust.subset.v1`. A construct is supported only when it compiles, runs natively and through WASM, and recovers from a single-frame resume at every durable boundary with the same observable result.

`engine_format_version` stays 1. This frontend adds no opcodes, heap tags, or durability rules.

Authoring imports durable operations from `trigora` or `tcc_rust_prelude`. The prelude exists so `cargo check` can typecheck the source. The lowerer does not execute the prelude bodies. A name is durable only when that import resolved it, including an alias (`use trigora::effect as fx`). `use trigora::*` is rejected. `trigora::effect(...)` is rejected. A helper, parameter, local, or pattern binding that reuses an imported durable name is rejected. A local `fn effect` that was not imported stays an ordinary helper. An imported durable name that is not awaited is rejected.

## Move rule

The owned domain is `f64`, `bool`, `String`, structs, enums, `Option<T>`, `Result<T, E>`, and `Vec<T>`. `T` and `E` are that same domain. `f64` and `bool` copy. Every other owned value moves. Assignment copies `f64` and `bool`. Compound arithmetic assignment (`+=` and the rest) is only for numeric locals and numeric fields. A moving local is moved with `let`, not `name = other`.

Compound values move in the source. The continuation stores them as heap refs. A move stores the destination and then stores `undefined` into the source. The lowerer emits no later read of that source. A field move is partial: `let name = job.name` stores `undefined` into `job.name` and leaves `job.score` readable. A second read of `job.name` is a moved-value error. A VM dump that still shows a ref in the destination is not source-level aliasing.

`cargo check` against the prelude owns move checking, types, and `match` exhaustiveness. The lowerer is a separate `syn` walk. It does not call rustc and it does not resolve types that are not visible in the syntax. A value whose type is not written at the binding needs a type ascription (`let approved: bool = ...`). A construct that needs HIR or MIR is rejected.

Frontend tests run `cargo check` before `compile`. A fixture `cargo check` rejects is not passed to the lowerer.

## Supported

- Exactly one `pub async fn main` with plain owned parameters. That function is the program entry. Its artifact name is `main`. `program.entry` is its function id. A private `async fn main` is a compile error. Argument binding is exact arity with no defaults
- Ordinary `fn` helpers. They are `Call`. They cannot await. A durable operation runs only while the entry frame is alone
- `let` / `let mut`, assignment, and early `return`
- `if` / `else`, `loop`, `while`, `for x in xs` over `Vec<T>`, and `for i in 0..n` as a counted `f64` loop. `i32` and every other integer type written in the source are rejected. `as f64` is a no-op on that loop binding
- Any number of structs and enums in one file. A struct field or enum payload is any owned value above. Structs are objects keyed by field names. Enums, `Option`, and `Result` are objects with compiler-only `$tag` and `$0` keys. A named variant stores its field names beside `$tag`. A tuple variant carries one payload at `$0`. `State::Pair(String, f64)` is rejected; the named-field form is the multi-value spelling. `Option<T>` and `Result<T, E>` are built-in enums (`Some`/`None`, `Ok`/`Err`) and lower through that same path. A struct or enum that reaches itself through a field, payload, `Vec`, `Option`, or `Result` is rejected. There is no `Box`
- Struct literals and variant constructors. Every field is required. `..rest` is rejected
- One-level enum, `Option`, and `Result` patterns. No guards, or-patterns, or rest. Nested patterns such as `Some(Job { count })` are a later gap. Bind the payload, then read its fields from that local
- `?` on a subset `Result`: a tag test plus `return Err(e)`. It does not emit `Throw`. Durable operations still produce `Result<T, String>`
- `Vec::new` needs a type ascription or a turbofish. `push` and `len` work for every `Vec<T>`. `for` consumes the vector and moves each element into the binding. `xs[i]` copies `f64` and `bool` only. Indexing a moving element would borrow, so it is rejected. `String` is an owned value with no mutation method
- `move` closures for ordinary computation, through the existing cell, environment, and `CallClosure` instructions. Capturing by reference is rejected. The `move` keyword is required when a closure captures
- Durable intrinsics, entry frame only, and only through a resolved import. Names are string literals (`"approved"`). The prelude types those parameters as `&'static str`. That is not general borrow support: `&` and `&mut` anywhere else, including `&T` and `&mut T` in user code, are rejected. An effect closure is typechecked and omitted from the artifact. The host supplies the result. Rust always emits `Effect` with `has_input: true`, including a closure that captures nothing: an empty object, then the key. Each `move` capture is stored under its binding name, in first-seen source order. `f64` and `bool` copy. `String`, structs, enums, `Option`, `Result`, and `Vec` move and clear the local. A capture without `move` is rejected. Those names are the Rust effect-bundle ABI shared by this frontend and the effect harness. They are not TCC object semantics and not a stable engine layout. `wait_for_event` and `invoke` take their result type from the ascription

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

With a span diagnostic: `&` / `&mut` outside the durable-name literal, lifetimes, user generics, traits, `impl`, macros, derives, iterator adapters, async helpers, generators, threads, `unsafe`, `Drop`, integer types, recursive types, indexing a moving `Vec` element, tuple variants with more than one payload, nested patterns, glob imports, qualified durable calls, shadowing an imported durable name, and imports other than `trigora` and `tcc_rust_prelude`. Integer types are outside this subset even though they are ordinary Rust: every number is `f64`. Nested patterns stay rejected.
