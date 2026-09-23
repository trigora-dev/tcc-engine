#![allow(unused)]

include!("../../../fixtures/move_job.rs");

fn main() {
    let _future = run(Job { count: 1.0 });
}
