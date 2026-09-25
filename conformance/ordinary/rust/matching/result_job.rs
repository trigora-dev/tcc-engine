pub struct Job {
    name: String,
    score: f64,
}

fn decode(score: f64) -> Result<Job, String> {
    if score > 0.0 {
        Ok(Job {
            name: String::from("research"),
            score,
        })
    } else {
        Err(String::from("missing"))
    }
}

pub async fn main(score: f64) -> Result<f64, String> {
    let job = decode(score)?;
    let name = job.name;
    if name == "research" {
        Ok(job.score)
    } else {
        Err(String::from("wrong"))
    }
}
