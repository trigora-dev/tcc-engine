#![allow(unused)]

mod program {
    include!("../../../fixtures/job.rs");
}

fn main() {
    let _future = program::main(program::Job { count: 0.0 });
}
