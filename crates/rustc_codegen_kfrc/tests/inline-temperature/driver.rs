fn main() {
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
