#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/ownership/move_string.rs");
}

fn main() {
    let _future = program::main(String::new());
}
