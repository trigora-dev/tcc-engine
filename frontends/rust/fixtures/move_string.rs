use tcc_rust_prelude::wait_for_event;

async fn run(name: String) -> Result<String, String> {
    let b = name;
    let _approved: bool = wait_for_event("go").await?;
    Ok(b)
}
