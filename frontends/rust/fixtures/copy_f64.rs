use tcc_rust_prelude::wait_for_event;

pub async fn main(a: f64) -> Result<f64, String> {
    let mut b = a;
    b += 1.0;
    let _approved: bool = wait_for_event("go").await?;
    let _keep = b;
    Ok(a)
}
