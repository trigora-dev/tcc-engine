#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/matching/result_question.rs");
}

fn main() {
    let _future = program::main(1.0);
}
