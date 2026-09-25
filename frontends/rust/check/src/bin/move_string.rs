#![allow(unused)]

mod program {
    include!("../../../fixtures/move_string.rs");
}

fn main() {
    let _future = program::main(String::from("hello"));
}
