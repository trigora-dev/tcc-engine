#![allow(unused)]

include!("../../../../../conformance/ordinary/rust/ownership/move_struct.rs");

fn main() {
    let _future = run(Job { count: 0.0 });
}
