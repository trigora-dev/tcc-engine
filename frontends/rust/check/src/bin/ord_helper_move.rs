#![allow(unused)]

include!("../../../../../conformance/ordinary/rust/ownership/helper_move.rs");

fn main() {
    let _future = run(Job { count: 0.0 });
}
