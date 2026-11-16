fn main() {
    if std::env::args().nth(1).as_deref() == Some("cold-abi") {
        inline_temperature::guarded_abi(
            -117,
            0xff37,
            0x8123_4567_89ab_cdef,
            0xffc1_2345,
            0xfff8_1234_5678_9abc,
            true,
        );
        panic!("cold ABI guard unexpectedly returned");
    }
    if std::env::args().nth(1).as_deref() == Some("cold") {
        let value = inline_temperature::guarded_loop(41, 0, 1);
        panic!("cold guard unexpectedly returned {value}");
    }
    let mut checksum = 0u64;
    for seed in 0..=u16::MAX {
        for key in [0, 1, 3, 37, u64::MAX, 0x8123_4567_89ab_cdef] {
            let seed = u64::from(seed);
            checksum = checksum.rotate_left(3) ^ inline_temperature::ordinary(seed, key);
            for count in [0, 1, 2, 7, 31] {
                checksum = checksum.rotate_left(3) ^ inline_temperature::cyclic(seed, key, count);
                if key != 0 {
                    checksum = checksum.rotate_left(3)
                        ^ inline_temperature::guarded_loop(seed, key, count);
                }
            }
        }
    }
    println!("{checksum:016x}");
}

#[unsafe(no_mangle)]
pub extern "C" fn observe_cold_arguments(
    a: i8,
    b: u16,
    c: u64,
    d: u64,
    e: u64,
    f: u64,
    stack: u64,
    x_bits: u32,
    y_bits: u64,
) -> u64 {
    assert_eq!(a, -117);
    assert_eq!(b, 0xff37);
    assert_eq!(d, c ^ 3);
    assert_eq!(e, c ^ 5);
    assert_eq!(f, c ^ 7);
    assert_eq!(stack, c ^ 11);
    assert_eq!(x_bits, 0xffc1_2345);
    assert_eq!(y_bits, 0xfff8_1234_5678_9abc);
    (a as u64) ^ u64::from(b) ^ c ^ d ^ e ^ f ^ stack ^ u64::from(x_bits) ^ y_bits
}
