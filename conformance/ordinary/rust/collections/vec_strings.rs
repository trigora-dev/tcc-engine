use tcc_rust_prelude::Vec;

async fn run() -> f64 {
    let mut names: Vec<String> = Vec::new();
    names.push(String::from("ab"));
    names.push(String::from("owned"));
    let mut count = 0.0;
    for value in names {
        count += if value == "owned" { 1.0 } else { 0.0 };
    }
    count
}
