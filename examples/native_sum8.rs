fn sum8(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64) -> i64 {
    return a + b + c + d + e + f + g + h;
}

fn example_result() -> i64 {
    return sum8(1, 2, 3, 4, 5, 6, 7, 8);
}

fn main() { println!("{}", example_result()); }
