use tcc_rust_prelude::Vec;

pub struct Job {
    name: String,
    active: bool,
    score: f64,
}

pub async fn main() -> f64 {
    let mut jobs: Vec<Job> = Vec::new();
    jobs.push(Job {
        name: String::from("a"),
        active: true,
        score: 10.0,
    });
    jobs.push(Job {
        name: String::from("b"),
        active: false,
        score: 32.0,
    });
    let mut total = 0.0;
    for job in jobs {
        total += if job.active { job.score } else { 0.0 };
    }
    total
}
