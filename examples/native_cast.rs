fn convert(value: i64) -> u64 {
    return value as u64;
}

fn example_result() -> i64 {
    let highest_bit = convert(-1) >> 63;
    return (highest_bit == 1) as i64 + 41;
}

fn main() { println!("{}", example_result()); }
