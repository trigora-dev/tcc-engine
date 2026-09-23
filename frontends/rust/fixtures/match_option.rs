async fn run(value: Option<f64>) -> f64 {
    match value {
        Some(n) => n,
        None => 0.0,
    }
}
