#![allow(unused, clippy::manual_unwrap_or)]

include!("../../../../../conformance/ordinary/rust/matching/match_option.rs");

fn main() {
    let _future = run(Some(0.0));
}
