#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/durability/effect_then_wait.rs");
}

fn main() {
    let _future = program::main();
}
