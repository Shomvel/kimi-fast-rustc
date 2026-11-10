use std::hint::black_box;
fn main() {
    let mut state = 0x123456789abcdef0u64;
    for i in 0..4096 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x = black_box(state);
        assert_eq!(kernels::nested_add(x), x.wrapping_add(42));
        assert_eq!(kernels::nested_mul(x), x.wrapping_mul(21));
        assert_eq!(kernels::nested_and(x), x & 0x0f000f);
        assert_eq!(kernels::nested_or(x), x | 0xff00);
        assert_eq!(kernels::nested_xor(x), x ^ 0xff00);
        assert_eq!(kernels::reversed_add(x, i), 0);
    }
    println!("PASS: 4096 inputs x 6 kernels");
}
