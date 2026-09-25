fn double(value: f64) -> f64 {
    value * 2.0
}

pub async fn main(value: f64) -> f64 {
    double(value)
}
