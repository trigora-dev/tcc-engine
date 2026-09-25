pub async fn main() -> f64 {
    let n = 4;
    let mut total = 0.0;
    for i in 0..n {
        total += i as f64;
    }
    total
}
