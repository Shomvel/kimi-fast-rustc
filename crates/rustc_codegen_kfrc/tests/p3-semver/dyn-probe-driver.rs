fn main() {
    for number in [0, 1, 127, u32::MAX as u64] {
        println!("{}", dyn_probe::debug_values(number));
        println!("{}", dyn_probe::virtual_debug(&number));
        println!("{}", dyn_probe::captured_call(number));
    }
}
