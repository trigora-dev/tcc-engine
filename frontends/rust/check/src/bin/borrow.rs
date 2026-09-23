#![allow(unused)]

include!("../../../fixtures/borrow.rs");

fn main() {
    let _future = run(Job { count: 1.0 });
}
