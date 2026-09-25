pub struct Job {
    pub count: f64,
}

fn pass(job: Job) -> Job {
    job
}

pub async fn main(job: Job) -> f64 {
    let moved = pass(job);
    moved.count
}
