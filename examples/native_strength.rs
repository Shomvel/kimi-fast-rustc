fn scale(value: i64) -> i64 {
    return value * 8;
}

fn reduce(value: u64) -> u64 {
    return value / 8;
}

fn example_result() -> i64 {
    return scale(5) + reduce(16) as i64;
}

fn main() { println!("{}", example_result()); }
