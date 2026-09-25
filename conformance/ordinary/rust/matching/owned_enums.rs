use tcc_rust_prelude::Vec;

enum Decision {
    Yes,
    No,
}

pub enum State {
    Ready,
    Failed(String),
    Pair { name: String, score: f64 },
}

pub async fn main() -> f64 {
    let failed = State::Failed(String::from("timeout"));
    let from_failed = match failed {
        State::Ready => 0.0,
        State::Failed(reason) => {
            if reason == "timeout" {
                1.0
            } else {
                0.0
            }
        }
        State::Pair { name, score } => {
            if name == "x" {
                score
            } else {
                0.0
            }
        }
    };
    let pair = State::Pair {
        name: String::from("x"),
        score: 40.0,
    };
    let from_pair = match pair {
        State::Ready => 0.0,
        State::Failed(reason) => {
            if reason == "no" {
                1.0
            } else {
                0.0
            }
        }
        State::Pair { name, score } => {
            if name == "x" {
                score
            } else {
                0.0
            }
        }
    };
    let decision = Decision::Yes;
    let from_decision = match decision {
        Decision::Yes => 1.0,
        Decision::No => 0.0,
    };
    let mut flags: Vec<Option<f64>> = Vec::new();
    flags.push(Some(1.0));
    let mut extra = 0.0;
    for flag in flags {
        extra += match flag {
            Some(value) => value,
            None => 0.0,
        };
    }
    from_failed + from_pair + from_decision + extra
}
