use tcc_rust_prelude::invoke;

async fn run() -> Result<f64, String> {
    let total: f64 = invoke("child", (1.0, 2.0)).await?;
    Ok(total)
}
