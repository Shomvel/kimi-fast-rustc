fn sum() -> i64 {
    let mut total = 0;
    for i in 0..5 {
        total += i;
    }
    total + 32
}

fn example_result() -> i64 {
    sum()
}

fn main() { println!("{}", example_result()); }
