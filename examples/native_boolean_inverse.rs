fn example_result() -> i64 {
    let signed = !(6 < 4);
    let unsigned = !(0xffff_ffff_ffff_ffffu64 < 1);
    return if signed && unsigned { 42 } else { 0 };
}

fn main() { println!("{}", example_result()); }
