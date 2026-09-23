#![allow(unused)]

include!("../../../../../conformance/ordinary/rust/matching/match_enum.rs");

fn main() {
    let _future = run(State::Ready { count: 0.0 });
}
