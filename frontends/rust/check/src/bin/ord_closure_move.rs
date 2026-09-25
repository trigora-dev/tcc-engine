#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/closures/closure_move.rs");
}

fn main() {
    let _future = program::main(1.0);
}
