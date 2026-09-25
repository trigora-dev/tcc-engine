#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/ownership/closure_capture.rs");
}

fn main() {
    let _future = program::main(String::new());
}
