pub async fn main(value: f64) -> f64 {
    let add = move |x: f64| x + value;
    add(2.0)
}
