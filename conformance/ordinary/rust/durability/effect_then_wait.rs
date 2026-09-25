use tcc_rust_prelude::{effect, wait_for_event};

pub async fn main() -> Result<f64, String> {
    let left: f64 = effect("left", || 2.0).await?;
    let right = left + 3.0;
    let _go: bool = wait_for_event("go").await?;
    Ok(right)
}
