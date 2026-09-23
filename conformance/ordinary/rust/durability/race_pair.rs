use tcc_rust_prelude::{effect, race};

async fn run() -> Result<f64, String> {
    let winner: f64 = race(effect("a", || 0.0), effect("b", || 0.0)).await?;
    Ok(winner)
}
