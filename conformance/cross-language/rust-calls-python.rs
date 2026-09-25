use tcc_rust_prelude::invoke;

pub async fn main() -> Result<f64, String> {
    let value: f64 = invoke("child", 2.0).await?;
    Ok(value + 1.0)
}
