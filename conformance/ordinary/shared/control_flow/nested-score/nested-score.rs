fn score(a: f64, b: f64, c: f64) -> f64 {
    (a + b) * c - a / 2.0
}

async fn run() -> f64 {
    score(4.0, 2.0, 3.0)
}
