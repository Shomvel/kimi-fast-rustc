fn is_positive(value: i64) -> bool {
    return value > 0;
}

fn example_result() -> i64 {
    let positive = is_positive(42);
    let valid = positive && !false;
    return if valid { 42 } else { 0 };
}

fn main() { println!("{}", example_result()); }
