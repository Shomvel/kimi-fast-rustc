fn greater(value: i64) -> i64 {
    return if 2 < value { 42 } else { 0 };
}

fn example_result() -> i64 {
    return greater(3);
}

fn main() { println!("{}", example_result()); }
