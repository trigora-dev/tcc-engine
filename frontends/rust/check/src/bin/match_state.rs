#![allow(unused)]

include!("../../../fixtures/match_state.rs");

fn main() {
    let _future = run(State::Ready { count: 1.0 });
    let _future = run(State::Done);
}
