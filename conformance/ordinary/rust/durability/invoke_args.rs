use tcc_rust_prelude::invoke;

pub async fn main() -> Result<f64, String> {
    let total: f64 = invoke("child", (1.0, 2.0)).await?;
    Ok(total)
}
