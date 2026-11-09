fn example_result() -> i64 {
    let mut value = false;
    let mut changes = 0;
    let mut index = 0;
    while index < 4 {
        let next = !value;
        if next != value {
            changes += 1;
        }
        value = next;
        index += 1;
    }
    let unchanged = value == false;
    let wrong_count = changes != 4;
    return if unchanged && !wrong_count { 42 } else { 0 };
}

fn main() { println!("{}", example_result()); }
