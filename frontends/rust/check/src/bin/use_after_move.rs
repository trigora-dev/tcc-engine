async fn run(name: String) -> String {
    let moved = name;
    let _again = moved;
    moved
}

fn main() {
    let _future = run(String::new());
}
