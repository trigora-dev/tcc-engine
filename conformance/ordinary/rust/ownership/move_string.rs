use tcc_rust_prelude::wait_for_event;

pub async fn main(name: String) -> Result<String, String> {
    let moved = name;
    let _approved: bool = wait_for_event("go").await?;
    Ok(moved)
}
