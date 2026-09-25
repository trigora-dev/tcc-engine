#![allow(unused, clippy::manual_unwrap_or)]

mod program {
    include!("../../../fixtures/match_option.rs");
}

fn main() {
    let _future = program::main(Some(1.0));
    let _future = program::main(None);
}
