#![allow(unused)]

mod program {
    include!("../../../fixtures/match_state.rs");
}

fn main() {
    let _future = program::main(program::State::Ready { count: 1.0 });
    let _future = program::main(program::State::Done);
}
