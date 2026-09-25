pub async fn main(value: Option<f64>) -> f64 {
    match value {
        Some(n) => n,
        None => 0.0,
    }
}
