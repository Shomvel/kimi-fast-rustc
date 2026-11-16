extern crate lane_promotion as subject;

fn loop_expected(count: u64, mut left: u64, mut right: u64) -> u64 {
    for index in 0..count {
        let old = (left, right);
        left = old.1.wrapping_add(index);
        right = old.0 ^ old.1.rotate_left(3);
    }
    left ^ right
}

fn main() {
    let bytes: Vec<_> = (0..=255).map(|n| n as u8).collect();
    let mut checksum = 0u64;
    for value in 0..=u16::MAX {
        let value = u64::from(value);
        let a = value.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let b = !a;
        let mut expected = if value & 1 == 0 { a } else { b };
        if value & 2 != 0 {
            expected = expected.wrapping_add(37);
        }
        if value & 1 != 0 {
            expected = expected.rotate_left(7);
        }
        assert_eq!(subject::branch_fields(value, a, b), expected);
        assert_eq!(
            subject::loop_fields(value & 31, a, b),
            loop_expected(value & 31, a, b)
        );
        assert_eq!(
            subject::escaped_fields(value),
            37u64.wrapping_add(value) ^ (91 ^ value)
        );
        checksum = checksum.wrapping_add(expected);
    }
    for length in 0..=bytes.len() {
        for width in 1..=32 {
            let mut expected = 0u64;
            for (index, chunk) in bytes[..length].chunks_exact(width).enumerate() {
                expected = expected.wrapping_add((index as u64).wrapping_mul(chunk.len() as u64));
                for byte in chunk {
                    expected = expected.rotate_left(1) ^ u64::from(*byte);
                }
            }
            assert_eq!(subject::chunk_fields(&bytes[..length], width), expected);
            checksum = checksum.wrapping_add(expected);
        }
    }
    println!("lane differential {checksum}");
}
