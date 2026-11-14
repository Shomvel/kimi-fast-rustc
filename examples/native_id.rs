fn id(x: i64) -> i64 {
    return x;
}

fn example_result() -> i64 {
    return id(42);
}

fn main() { println!("{}", example_result()); }
