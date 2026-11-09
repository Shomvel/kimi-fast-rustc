fn example_result() -> i64 {
    let value = 5;
    let shifted = (value << 4) | 2;
    let masked = shifted & 126;
    let result = masked ^ 80;
    let negated = -result;
    return (negated >> 1) + 43;
}

fn main() { println!("{}", example_result()); }
