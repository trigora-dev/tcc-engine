use tcc_rust_prelude::wait_for_event;

pub struct Job {
    pub count: f64,
}

pub async fn main(mut job: Job) -> Result<f64, String> {
    let n = 3;
    for i in 0..n {
        job.count += i as f64;
    }
    let approved = wait_for_event("approved").await?;
    if approved {
        Ok(job.count)
    } else {
        Err("rejected".into())
    }
}
