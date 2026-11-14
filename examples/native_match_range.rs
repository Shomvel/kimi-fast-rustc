fn example_result() -> i64 {
    let value = 42;
    return match value {
        -10..=10 => 0,
        40..=50 if value % 2 == 0 => 42,
        _ => -1,
    };
}

fn main() { println!("{}", example_result()); }
