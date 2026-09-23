fn classify(n: f64) -> String {
    if n < 0.0 {
        String::from("neg")
    } else if n == 0.0 {
        String::from("zero")
    } else {
        String::from("pos")
    }
}

async fn run() -> String {
    classify(-3.0)
}
