enum State {
    Ready { count: f64 },
    Done,
}

async fn run(state: State) -> f64 {
    match state {
        State::Ready { count } => count,
        State::Done => 0.0,
    }
}
