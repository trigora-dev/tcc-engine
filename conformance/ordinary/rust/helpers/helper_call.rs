fn double(value: f64) -> f64 {
    value * 2.0
}

async fn run(value: f64) -> f64 {
    double(value)
}
