fn normalize(mut value: i64) -> i64 {
    value += 5;
    value *= 3;
    if value >= 100 {
        value = 100;
    } else if value >= 50 {
        value -= 21;
    } else if value < 0 {
        value = 0;
    }
    return value;
}

fn example_result() -> i64 {
    return normalize(16);
}

fn main() { println!("{}", example_result()); }
