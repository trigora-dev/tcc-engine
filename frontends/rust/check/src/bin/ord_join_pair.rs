#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/durability/join_pair.rs");
}

fn main() {
    let _future = program::main();
}
