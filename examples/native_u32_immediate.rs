fn example_result() -> u64 {
    let value = 0x8000_0000u64;
    return if value > 0x7fff_ffffu64 { 42u64 } else { 0 };
}

fn main() { println!("{}", example_result()); }
