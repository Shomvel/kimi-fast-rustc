fn example_result() -> i64 {
    let mut x = 40;
    for x in 0..3 {
        let x = x + 1;
        if x == 2 {
            let x = 100;
        }
    }
    let x = x + 1;
    let mut i = 0;
    while i < 1 {
        let x = 0;
        i += 1;
    }
    return x + 1;
}

fn main() { println!("{}", example_result()); }
