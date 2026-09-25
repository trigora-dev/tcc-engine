#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/values/copy_f64.rs");
}

fn main() {
    let _future = program::main(0.0);
}
