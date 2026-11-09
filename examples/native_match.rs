fn example_result() -> i64 {
    return match 2 {
        0 => 0,
        1 => 21,
        2 => { 40 + 2 },
        _ => -1,
    };
}

fn main() { println!("{}", example_result()); }
