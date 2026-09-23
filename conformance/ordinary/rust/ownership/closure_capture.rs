async fn run(name: String) -> String {
    let read = move || name;
    read()
}
