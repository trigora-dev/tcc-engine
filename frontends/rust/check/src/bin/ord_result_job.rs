#![allow(unused)]

include!("../../../../../conformance/ordinary/rust/matching/result_job.rs");

fn main() {
    let _future = run(42.0);
    let _missing = run(0.0);
}
