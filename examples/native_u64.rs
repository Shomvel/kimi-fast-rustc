fn compute(value: u64) -> u64 {
    return (value >> 63) + (value % 3) + 41;
}

fn example_result() -> u64 {
    return compute(18446744073709551615u64);
}

fn main() { println!("{}", example_result()); }
