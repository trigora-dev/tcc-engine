struct Job {
    count: f64,
}

async fn run(job: Job) -> f64 {
    let x = &job;
    x.count
}
