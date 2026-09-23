#![allow(unused)]

include!("../../../fixtures/move_string.rs");

fn main() {
    let _future = run(String::from("hello"));
}
