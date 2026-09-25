#![allow(unused)]

mod program {
    include!("../../../fixtures/move_job.rs");
}

fn main() {
    let _future = program::main(program::Job { count: 1.0 });
}
