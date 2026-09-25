pub async fn main(name: String) -> String {
    let read = move || name;
    read()
}
