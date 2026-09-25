#![allow(unused)]

mod program {
    include!("../../../fixtures/copy_f64.rs");
}

fn main() {
    let _future = program::main(1.0);
}
