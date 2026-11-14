fn answer() -> i64 {
    return 42;
}

fn example_result() -> i64 {
    return answer();
}

fn main() { println!("{}", example_result()); }
