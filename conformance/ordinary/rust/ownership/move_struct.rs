use tcc_rust_prelude::wait_for_event;

struct Job {
    count: f64,
}

async fn run(job: Job) -> Result<f64, String> {
    let mut moved = job;
    moved.count += 1.0;
    let _approved: bool = wait_for_event("go").await?;
    Ok(moved.count)
}
