use tcc_rust_prelude::Vec;

async fn run() -> f64 {
    let mut xs: Vec<f64> = Vec::new();
    xs.push(10.0);
    xs.push(22.0);
    let first = xs[0.0];
    let mut total = first;
    for value in xs {
        total += value;
    }
    total
}
