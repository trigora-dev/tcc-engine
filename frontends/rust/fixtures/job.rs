use tcc_rust_prelude::wait_for_event;

struct Job {
    count: f64,
}

async fn run(mut job: Job) -> Result<f64, String> {
    for i in 0..3 {
        job.count += i as f64;
    }
    let approved = wait_for_event("approved").await?;
    if approved {
        Ok(job.count)
    } else {
        Err("rejected".into())
    }
}
