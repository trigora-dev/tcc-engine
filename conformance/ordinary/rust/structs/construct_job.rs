struct Note {
    flag: bool,
}

pub struct Job {
    name: String,
    active: bool,
    score: f64,
    note: Note,
}

pub async fn main() -> f64 {
    let job = Job {
        name: String::from("research"),
        active: true,
        score: 42.0,
        note: Note { flag: true },
    };
    let name = job.name;
    let score = job.score;
    let note = job.note;
    if name == "research" && job.active && note.flag {
        score
    } else {
        0.0
    }
}
