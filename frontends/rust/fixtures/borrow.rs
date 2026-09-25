pub struct Job {
    pub count: f64,
}

pub async fn main(job: Job) -> f64 {
    let x = &job;
    x.count
}
