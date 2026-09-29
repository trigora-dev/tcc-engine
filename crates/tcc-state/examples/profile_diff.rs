// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! Internal native micro-profile: `cargo run -p tcc-state --release --example profile_diff`.
use std::hint::black_box;
use std::time::Instant;
use tcc_state::{persist_intent, Continuation, Value};

fn main() {
    const ITERATIONS: usize = 100_000;
    for locals in [2, 8, 32] {
        let mut base = Continuation::start("profile", "hash", 1, "ts.subset.v1", 0, locals);
        for (i, slot) in base.frames[0].locals.iter_mut().enumerate() {
            *slot = Value::Number(i as f64);
        }
        let mut current = base.clone();
        current.frames[0].pc = 2;
        current.frames[0].locals[0] = Value::Number(42.0);
        let started = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(persist_intent(
                black_box(Some(&base)),
                black_box(&current),
                0,
            ));
        }
        println!(
            "locals={locals} persist_intent_ns={:.1}",
            started.elapsed().as_nanos() as f64 / ITERATIONS as f64
        );
    }
}
