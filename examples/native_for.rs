fn start() -> usize {
    return 1;
}

fn end() -> usize {
    return 6;
}

fn example_result() -> u64 {
    let mut total = 0usize;
    for i in start()..end() {
        if i % 2 == 0 {
            continue;
        }
        total += i;
    }
    return (total + 33) as u64;
}

fn main() { println!("{}", example_result()); }
