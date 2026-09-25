#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/ownership/move_struct.rs");
}

fn main() {
    let _future = program::main(program::Job { count: 0.0 });
}
