#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/durability/invoke_args.rs");
}

fn main() {
    let _future = program::main();
}
