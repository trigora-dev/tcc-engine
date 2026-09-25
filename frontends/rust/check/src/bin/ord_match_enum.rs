#![allow(unused)]

mod program {
    include!("../../../../../conformance/ordinary/rust/matching/match_enum.rs");
}

fn main() {
    let _future = program::main(program::State::Ready { count: 0.0 });
}
