#![allow(unused, clippy::manual_unwrap_or)]

mod program {
    include!("../../../../../conformance/ordinary/rust/matching/match_option.rs");
}

fn main() {
    let _future = program::main(Some(0.0));
}
