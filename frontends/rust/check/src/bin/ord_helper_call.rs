#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/helpers/helper_call.rs");
}

fn main() {
    let _future = program::main(1.0);
}
