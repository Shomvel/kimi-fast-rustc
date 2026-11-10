use std::hint::black_box;
fn main() {
    let mut state=0x123456789abcdef0u64;
    for i in 0..4096u64 {
        state=state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x=black_box(state);
        let byte=black_box(i as u8);
        assert_eq!(kernels::byte_shift(byte), u64::from(byte >> 2));
        assert_eq!(kernels::low_shift(x), x << 3);
        assert_eq!(kernels::invert_mask(byte), !(byte & 15));
        assert_eq!(kernels::sign_shift(byte as i8), ((byte | 128) as i8) >> 3);
    }
    println!("PASS: 4096 inputs x 4 kernels");
}
