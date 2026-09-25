use tcc_rust_prelude::{effect, wait_for_event};

pub async fn main() -> Result<f64, String> {
    let result: f64 = effect("generate", || 99.0).await?;
    let _approval: String = wait_for_event("approved").await?;
    Ok(result + 1.0)
}
