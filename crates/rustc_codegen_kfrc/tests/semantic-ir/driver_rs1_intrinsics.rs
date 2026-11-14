use semantic_values::rs1_intrinsics_probe::*;

macro_rules! check_width {
    ($ty:ty, $counts:ident, $reverse:ident, $rotate:ident, $saturate:ident) => {{
        let bits = <$ty>::BITS;
        let high = 1u64 << (bits - 1);
        let mask = u64::MAX >> (64 - bits);
        let values = [0, 1, 3, high - 1, high, mask - 1, mask];
        let mut checksum = 0u64;
        for raw in values {
            let value = raw as $ty;
            assert_eq!(
                $counts(value),
                (
                    value.count_ones(),
                    value.leading_zeros(),
                    value.trailing_zeros()
                )
            );
            let reversed = $reverse(value);
            assert_eq!(reversed, (value.swap_bytes(), value.reverse_bits()));
            checksum = checksum.wrapping_add(reversed.0 as u64 ^ reversed.1 as u64);
            for amount in [0, 1, bits - 1, bits, bits + 1, 1000, u32::MAX] {
                let rotated = $rotate(value, amount);
                assert_eq!(
                    rotated,
                    (value.rotate_left(amount), value.rotate_right(amount))
                );
                checksum = checksum.wrapping_add(rotated.0 as u64 ^ rotated.1 as u64);
            }
            for other in values {
                let other = other as $ty;
                let saturated = $saturate(value, other);
                assert_eq!(
                    saturated,
                    (value.saturating_add(other), value.saturating_sub(other))
                );
                checksum = checksum.wrapping_add(saturated.0 as u64 ^ saturated.1 as u64);
            }
        }
        println!("rs1 {} {checksum}", stringify!($ty));
    }};
}

macro_rules! check_funnel {
    ($ty:ty, $funnel:ident) => {{
        let bits = <$ty>::BITS;
        let mask = (1u128 << bits) - 1;
        let mut checksum = 0u64;
        for hi in [0, 1, 0xa5, 1u128 << (bits - 1), mask] {
            let lo = (hi ^ mask) & mask;
            let pair = (hi << bits) | lo;
            for amount in [0, 1, bits / 2, bits - 1] {
                // Every invocation satisfies the unchecked amount contract.
                let actual = unsafe { $funnel(hi as $ty, lo as $ty, amount) };
                let expected = (
                    ((pair >> (bits - amount)) & mask) as $ty,
                    ((pair >> amount) & mask) as $ty,
                );
                assert_eq!(actual, expected);
                checksum = checksum.wrapping_add(actual.0 as u64 ^ actual.1 as u64);
            }
        }
        println!("rs1-funnel {} {checksum}", stringify!($ty));
    }};
}

pub fn check() {
    check_width!(u8, counts_u8, reverse_u8, rotate_u8, saturate_u8);
    check_width!(u16, counts_u16, reverse_u16, rotate_u16, saturate_u16);
    check_width!(u32, counts_u32, reverse_u32, rotate_u32, saturate_u32);
    check_width!(u64, counts_u64, reverse_u64, rotate_u64, saturate_u64);
    check_width!(
        usize,
        counts_usize,
        reverse_usize,
        rotate_usize,
        saturate_usize
    );
    check_width!(i8, counts_i8, reverse_i8, rotate_i8, saturate_i8);
    check_width!(i16, counts_i16, reverse_i16, rotate_i16, saturate_i16);
    check_width!(i32, counts_i32, reverse_i32, rotate_i32, saturate_i32);
    check_width!(i64, counts_i64, reverse_i64, rotate_i64, saturate_i64);
    check_width!(
        isize,
        counts_isize,
        reverse_isize,
        rotate_isize,
        saturate_isize
    );
    check_funnel!(u8, funnel_u8);
    check_funnel!(u16, funnel_u16);
    check_funnel!(u32, funnel_u32);
    check_funnel!(u64, funnel_u64);
    check_funnel!(usize, funnel_usize);
    for value in [1u64, 2, 3, 0x80, 1 << 31, 1 << 63, u64::MAX] {
        assert_eq!(
            counts_nonzero(core::num::NonZeroU64::new(value).unwrap()),
            (value.leading_zeros(), value.trailing_zeros())
        );
    }
    assert_eq!(constants(), (8, 0x0807060504030201, 0x1e6a2c48, i16::MAX));
    for value in [0u64, 1, 127, 128, 16383, 16384, 1 << 63, u64::MAX] {
        let mut expected = 1;
        let mut remaining = value;
        while remaining >= 128 {
            expected += 1;
            remaining >>= 7;
        }
        assert_eq!(bytes_for_varint(value), expected);
        assert_eq!(inline_len(value), value.leading_zeros() / 8);
        for amount in [0, 1, 63, 64, u32::MAX] {
            assert_eq!(
                definition_probe(value, amount),
                u64::from(value.count_ones())
                    ^ value.rotate_left(amount)
                    ^ value.saturating_add(17)
            );
        }
    }
    println!("rs1-constants-and-helpers ok");
}
