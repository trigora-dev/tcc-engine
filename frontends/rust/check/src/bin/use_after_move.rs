mod program {
    pub async fn main(name: String) -> String {
        let moved = name;
        let _again = moved;
        moved
    }
}

fn main() {
    let _future = program::main(String::new());
}
