use tcc_rust_prelude::{effect, wait_for_event};

async fn run() -> Result<f64, String> {
    let result: f64 = effect("generate", || 42.0).await?;
    let _approval: String = wait_for_event("approved").await?;
    Ok(result)
}
