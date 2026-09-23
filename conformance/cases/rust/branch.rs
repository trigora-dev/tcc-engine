use tcc_rust_prelude::{effect, wait_for_event};

async fn run() -> Result<String, String> {
    let flag: f64 = effect("generate", || 1.0).await?;
    if flag != 0.0 {
        let approval: String = wait_for_event("approved").await?;
        Ok(approval)
    } else {
        Err(String::from("zero"))
    }
}
