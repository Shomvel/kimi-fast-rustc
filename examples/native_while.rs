fn sum_to(limit: i64) -> i64 {
    let mut sum = 0;
    let mut i = 1;
    while i <= limit {
        sum = sum + i;
        i = i + 1;
    }
    return sum;
}

fn example_result() -> i64 {
    return sum_to(10);
}

fn main() { println!("{}", example_result()); }
