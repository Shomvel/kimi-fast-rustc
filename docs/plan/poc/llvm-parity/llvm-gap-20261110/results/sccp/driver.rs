use std::hint::black_box;
fn main() {
    for i in 0..4096u64 {
        let flag = black_box(i & 1 == 0);
        let n = black_box(i % 32);
        assert_eq!(kernels::constant_merge(flag), 42);
        assert_eq!(kernels::constant_branch(flag), 42);
        assert_eq!(kernels::narrow_merge(flag), 1);
        assert_eq!(kernels::changing_loop(n), n + 42);
        assert_eq!(kernels::dynamic_merge(flag, i), if flag { 8 } else { i + 1 });
    }
    println!("PASS: 4096 inputs x 5 kernels");
}
