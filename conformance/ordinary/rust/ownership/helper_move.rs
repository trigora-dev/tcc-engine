struct Job {
    count: f64,
}

fn pass(job: Job) -> Job {
    job
}

async fn run(job: Job) -> f64 {
    let moved = pass(job);
    moved.count
}
