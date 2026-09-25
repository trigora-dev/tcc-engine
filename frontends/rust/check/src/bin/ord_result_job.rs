#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/matching/result_job.rs");
}

fn main() {
    let _future = program::main(42.0);
    let _missing = program::main(0.0);
}
