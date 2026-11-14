fn spin(mut value: i64) {
    while value > 0 {
        value -= 1;
    }
    return;
}

fn answer() -> i64 {
    return 42;
}

fn example_result() -> i64 {
    spin(3);
    answer();
    1 + 2;
    3 < 4;
    return answer();
}

fn main() { println!("{}", example_result()); }
