use instcombine_top5::*;

fn absorb(checksum: &mut u64, value: u64) {
    *checksum = checksum.rotate_left(7) ^ value.wrapping_mul(0x9e37_79b9_7f4a_7c15);
}

fn scalar_case(value: u64, amount: u32, checksum: &mut u64) {
    let wide = u128::from(value);
    let low = value as u8;
    let signed_half = value as i16;
    let other = value.rotate_left(29) as i64;
    let pow = mul_pow2_u64(value);
    assert_eq!(pow, (wide * 4) as u64);
    absorb(checksum, pow);
    let checked = checked_mul_pow2(value);
    let checked_wide = wide * 4;
    let checked_expected = if checked_wide <= u128::from(u64::MAX) {
        Some(checked_wide as u64)
    } else {
        None
    };
    assert_eq!(checked, checked_expected);
    absorb(checksum, u64::from(checked.is_some()));
    absorb(checksum, checked.unwrap_or(0));
    let signed = mul_pow2_i64_sign(value as i64);
    assert_eq!(signed as u64, (wide * (1u128 << 63)) as u64);
    absorb(checksum, signed as u64);
    let byte = mul_pow2_u8(low);
    assert_eq!(u64::from(byte), (u64::from(low) * 128) & 255);
    absorb(checksum, u64::from(byte));
    let half = mul_pow2_i16(signed_half);
    assert_eq!(half as u16, ((i64::from(signed_half) * 16) & 65535) as u16);
    absorb(checksum, half as u16 as u64);

    let add = trunc_add(value);
    assert_eq!(
        u64::from(add),
        ((wide + 0xfeed_1234_5678_9abc) & 65535) as u64
    );
    absorb(checksum, u64::from(add));
    let sub = trunc_sub(value as u32);
    assert_eq!(sub, ((0x34u64 + 256 - u64::from(low)) & 255) as u8);
    absorb(checksum, u64::from(sub));
    let mul = trunc_mul(value);
    assert_eq!(mul, (wide * 0x1234_5678_9abc_def1) as u32);
    absorb(checksum, u64::from(mul));
    let logic = trunc_logic(value);
    assert_eq!(logic, ((value & 65535) ^ 0xabcd) as u16);
    absorb(checksum, u64::from(logic));
    let and = trunc_and(value);
    assert_eq!(and, ((value & 65535) & 0x0f3c) as u16);
    absorb(checksum, u64::from(and));
    let or = trunc_or(value);
    assert_eq!(or, ((value & 65535) | 0x30c3) as u16);
    absorb(checksum, u64::from(or));
    let extended = trunc_extended(signed_half, other);
    assert_eq!(
        extended,
        (i128::from(signed_half) + i128::from(other)) as i16
    );
    absorb(checksum, extended as u16 as u64);
    let shared = trunc_shared(value);
    let full = (wide + 0xfeed_1234_5678_9abc) as u64;
    assert_eq!(shared, (full as u16, full));
    absorb(checksum, shared.1 ^ u64::from(shared.0));

    let left = opposite_left(value);
    assert_eq!(left, value & ((1u64 << 51) - 1));
    absorb(checksum, left);
    let right = opposite_right(value as u32);
    assert_eq!(right, value as u32 & !127u32);
    absorb(checksum, u64::from(right));
    let over = opposite_overshift(low);
    assert_eq!(over, low & 127);
    absorb(checksum, u64::from(over));
    let shared = opposite_shared(value);
    assert_eq!(shared, (left, (wide * (1u128 << 13)) as u64));
    absorb(checksum, shared.0 ^ shared.1);

    let dynamic = mul_shift_factor_u64(value, amount);
    assert_eq!(dynamic, (wide * (1u128 << (amount % 64))) as u64);
    absorb(checksum, dynamic);
    let half = mul_shift_factor_i16(signed_half, amount);
    let half_expected = i64::from(signed_half) * (1i64 << (amount % 16));
    assert_eq!(half as u16, (half_expected & 65535) as u16);
    absorb(checksum, half as u16 as u64);
    let byte = mul_shift_factor_u8(low, amount);
    assert_eq!(
        u64::from(byte),
        (u64::from(low) * (1u64 << (amount % 8))) & 255
    );
    absorb(checksum, u64::from(byte));
    let shared = mul_shift_factor_shared(value, amount);
    assert_eq!(shared, (dynamic, 1u64 << (amount % 64)));
    absorb(checksum, shared.0 ^ shared.1);
}

fn byte_order(left: &[u8], right: &[u8]) -> i32 {
    assert_eq!(left.len(), right.len());
    for (&left, &right) in left.iter().zip(right) {
        if left != right {
            return i32::from(left) - i32::from(right);
        }
    }
    0
}

fn bytes_case(left: &[u8], right: &[u8], checksum: &mut u64) {
    assert_eq!(left.len(), right.len());
    let expected = byte_order(left, right);
    // All pointer pairs are inside live arrays, including equal/overlapping
    // ranges. Even a zero-length call receives non-null initialized storage.
    unsafe {
        let equal = memcmp_equal(left.as_ptr(), right.as_ptr(), left.len());
        let unequal = memcmp_unequal(left.as_ptr(), right.as_ptr(), left.len());
        assert_eq!(equal, expected == 0);
        assert_eq!(unequal, expected != 0);
        let ordering = memcmp_order(left.as_ptr(), right.as_ptr(), left.len());
        assert_eq!(ordering.signum(), expected.signum());
        let mut observed = 0x1234_5678;
        let stored_equal =
            memcmp_store_then_equal(left.as_ptr(), right.as_ptr(), left.len(), &mut observed);
        assert_eq!(stored_equal, expected == 0);
        assert_eq!(observed.signum(), expected.signum());
        absorb(checksum, u64::from(equal) | (u64::from(unequal) << 1));
        // memcmp only promises a sign, not the magnitude of a nonzero result.
        absorb(checksum, (ordering.signum() + 1) as u64);
        absorb(checksum, (observed.signum() + 1) as u64);
        for select_first in [false, true] {
            let result = memcmp_equality_phi(
                left.as_ptr(),
                left.as_ptr(),
                right.as_ptr(),
                left.len(),
                select_first,
            );
            assert_eq!(result, select_first || expected == 0);
            absorb(checksum, u64::from(result));
        }
        if left.len() >= 7 {
            let equal = memcmp_equal_seven(left.as_ptr(), right.as_ptr());
            assert_eq!(equal, byte_order(&left[..7], &right[..7]) == 0);
            absorb(checksum, u64::from(equal));
        }
        if left.len() >= 32 {
            let equal = memcmp_equal_32(left.as_ptr(), right.as_ptr());
            assert_eq!(equal, byte_order(&left[..32], &right[..32]) == 0);
            absorb(checksum, u64::from(equal));
        }
    }
}

fn main() {
    let mut checksum = 0u64;
    let mut scalar_cases = 0usize;
    for value in 0..=255 {
        for amount in 0..=255 {
            scalar_case(value, amount, &mut checksum);
            scalar_cases += 1;
        }
    }
    let amounts = [
        0,
        1,
        7,
        8,
        9,
        15,
        16,
        17,
        31,
        32,
        33,
        63,
        64,
        65,
        127,
        255,
        u32::MAX,
    ];
    for value in [0, 1, 127, 128, 255, 256, 65535, 1 << 31, 1 << 63, u64::MAX] {
        for amount in amounts {
            scalar_case(value, amount, &mut checksum);
            scalar_cases += 1;
        }
    }
    let mut state = 0x807f_ff00_55aa_fe01u64;
    for _ in 0..20_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        scalar_case(state, state.rotate_left(29) as u32, &mut checksum);
        scalar_cases += 1;
    }

    let mut left = [0u8; 257];
    for (index, byte) in left.iter_mut().enumerate() {
        *byte = ((index * 73 + 19) & 255) as u8;
    }
    let lengths = [
        0, 1, 2, 3, 7, 8, 15, 16, 17, 31, 32, 33, 63, 64, 65, 128, 256,
    ];
    let mut byte_cases = 0usize;
    for length in lengths {
        bytes_case(&left[..length], &left[..length], &mut checksum);
        byte_cases += 1;
        bytes_case(&left[..length], &left[1..length + 1], &mut checksum);
        byte_cases += 1;
        let mut positions = vec![0, length / 2, length.saturating_sub(1)];
        positions.sort_unstable();
        positions.dedup();
        for position in positions {
            if position >= length {
                continue;
            }
            let mut right = left;
            right[position] ^= 0x80;
            bytes_case(&left[..length], &right[..length], &mut checksum);
            bytes_case(&right[..length], &left[..length], &mut checksum);
            byte_cases += 2;
        }
    }
    for _ in 0..512 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let length = state as usize % 257;
        let mut right = left;
        if length != 0 {
            right[state.rotate_left(11) as usize % length] ^= 0x5a;
        }
        bytes_case(&left[..length], &right[..length], &mut checksum);
        byte_cases += 1;
    }
    println!(
        "instcombine top5: {scalar_cases} scalar cases and {byte_cases} byte cases passed; checksum={checksum:016x}"
    );
}
