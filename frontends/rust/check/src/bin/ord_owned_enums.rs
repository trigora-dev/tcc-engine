#![allow(unused, clippy::manual_unwrap_or)]

mod program {
    include!("../../../../../conformance/ordinary/rust/matching/owned_enums.rs");
}

fn main() {
    let _future = program::main();
}
