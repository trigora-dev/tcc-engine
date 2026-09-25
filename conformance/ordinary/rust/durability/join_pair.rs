use tcc_rust_prelude::{effect, join, Vec};

pub async fn main() -> Result<f64, String> {
    let values: Vec<f64> = join(effect("a", || 0.0), effect("b", || 0.0)).await?;
    Ok(values[0.0] + values[1.0])
}
