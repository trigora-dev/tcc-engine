fn parse(value: f64) -> Result<f64, String> {
    if value > 0.0 && value < 10.0 {
        Ok(value)
    } else {
        Err("bad".into())
    }
}

async fn run(value: f64) -> Result<f64, String> {
    let got = parse(value)?;
    Ok(got + 1.0)
}
