fn example_result() -> u64 {
    let mut total = 0usize;
    for i in 1..=6usize {
        if i % 2 == 0 {
            continue;
        }
        total += i;
    }
    return (total + 33) as u64;
}

fn main() { println!("{}", example_result()); }
