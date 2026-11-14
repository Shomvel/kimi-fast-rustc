fn example_result() -> i64 {
    let answer = loop {
        break 40 + 2;
    };
    answer
}

fn main() { println!("{}", example_result()); }
