#![allow(unused)]

include!("../../../../../conformance/ordinary/rust/ownership/move_string.rs");

fn main() {
    let _future = run(String::new());
}
