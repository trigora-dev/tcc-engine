use tcc_rust_prelude::wait_for_event;

pub struct Job {
    pub count: f64,
}

pub async fn main(job: Job) -> Result<f64, String> {
    let mut moved = job;
    moved.count += 1.0;
    let _approved: bool = wait_for_event("go").await?;
    Ok(moved.count)
}
