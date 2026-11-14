fn example_result() -> i64 {
    let mut total = 0;
    let mut i = 0;
    while i < 4 {
        let mut j = 0;
        while j < i {
            total = total + j;
            j = j + 1;
        }
        i = i + 1;
    }
    return total;
}

fn main() { println!("{}", example_result()); }
