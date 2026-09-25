use tcc_rust_prelude::{effect, join, Vec};

pub async fn main() -> Result<Vec<String>, String> {
    let values = join(
        effect("a", || String::from("left")),
        effect("b", || String::from("right")),
    )
    .await?;
    Ok(values)
}
