#![allow(unused)]

include!("../../../../../conformance/ordinary/rust/ownership/closure_capture.rs");

fn main() {
    let _future = run(String::new());
}
