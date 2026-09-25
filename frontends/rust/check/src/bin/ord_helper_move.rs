#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/ownership/helper_move.rs");
}

fn main() {
    let _future = program::main(program::Job { count: 0.0 });
}
