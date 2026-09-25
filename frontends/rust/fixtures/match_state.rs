pub enum State {
    Ready { count: f64 },
    Done,
}

pub async fn main(state: State) -> f64 {
    match state {
        State::Ready { count } => count,
        State::Done => 0.0,
    }
}
