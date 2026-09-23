#![allow(unused, clippy::manual_unwrap_or)]

include!("../../../fixtures/match_option.rs");

fn main() {
    let _future = run(Some(1.0));
    let _future = run(None);
}
