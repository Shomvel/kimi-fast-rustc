pub const ANSWER: i64 = (BASE + OFFSET) * 2;
const BASE: i64 = LIMIT - 1;
const OFFSET: i64 = 1;
const LIMIT: i64 = 21;
const CHECKED: bool = ANSWER == 42 && !false;

fn example_result() -> i64 {
    if CHECKED {
        return ANSWER;
    }
    return 0;
}

fn main() { println!("{}", example_result()); }
