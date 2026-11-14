use generic_scalar::*;

fn main() {
    let mut state = 0x807f_ff00_55aa_fe01u64;
    let mut checksum = 0u64;
    let characters = [
        '\0',
        'A',
        '中',
        '\u{d7ff}',
        '\u{e000}',
        '\u{1f680}',
        '\u{10ffff}',
    ];
    for i in 0..20_006 {
        let value = match i {
            0 => 0,
            1 => 1,
            2 => u64::MAX,
            3 => 1 << 63,
            4 => 0x807f_ff00_55aa_fe01,
            5 => 0x0102_0304_0506_0708,
            _ => {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            }
        };
        let other = value.rotate_left(29).wrapping_add(i as u64);
        let low = value & 255;
        let unsigned = unsigned_chain(value);
        let signed = signed_chain(value as i8);
        let truncated = sign_after_truncation(value);
        let demand = demanded_low(value);
        let shared = demanded_shared(value);
        let shifted = shifted_low(value as i64);
        let known = known_sign_bits(value as i8);
        let character = characters[i % characters.len()];
        let validity = scalar_validity(i % 2 == 0, character);
        let snapshot = fixed_array_snapshot(value, other);
        let nested = nested_fixed_array(value, other);
        let recurrence = fixed_count_recurrence(value, other);
        let descending = signed_countdown(value);
        let wrapping = wrapping_byte_count(value);
        assert_eq!(u64::from(unsigned), value & 65535);
        let byte_signed = if low < 128 {
            low as i64
        } else {
            low as i64 - 256
        };
        assert_eq!(signed, byte_signed);
        let half = value & 65535;
        let half_signed = if half < 32768 {
            half as i64
        } else {
            half as i64 - 65536
        };
        assert_eq!(truncated, half_signed);
        assert_eq!(demand, low * 3);
        assert_eq!(shared, (value | 0xabcd_0000) & 0xffff_ffff);
        assert_eq!(u64::from(shifted), (value / 2048) & 31);
        assert_eq!(known, ((low / 8) as i8) | -16);
        assert_eq!(
            validity,
            u64::from(i % 2 == 0) ^ u64::from(character as u32)
        );
        assert_eq!(snapshot, value ^ value.wrapping_add(other));
        assert_eq!(nested, other ^ value.wrapping_add(other));
        let mut expected_left = value;
        let mut expected_right = other;
        for step in 0..4 {
            let next = expected_left.wrapping_add(expected_right);
            expected_left = (expected_right << 7) ^ step;
            expected_right = next;
        }
        assert_eq!(recurrence, expected_left ^ expected_right);
        assert_eq!(descending, value.wrapping_add(2));
        assert_eq!(wrapping, value.wrapping_add(510));
        for condition in [false, true] {
            let partial = partial_redundancy(value, other, condition);
            let expected = if condition {
                value.wrapping_mul(14)
            } else {
                other.wrapping_mul(7)
            };
            assert_eq!(partial, expected);
            let two_phis = two_phi_redundancy(value, other, condition);
            let expected = if condition {
                value.wrapping_add(other).wrapping_mul(2)
            } else {
                other.wrapping_add(value ^ 9)
            };
            assert_eq!(two_phis, expected);
            checksum = checksum.wrapping_add(partial ^ two_phis);
        }
        checksum = checksum.rotate_left(7)
            ^ u64::from(unsigned)
            ^ signed as u64
            ^ truncated as u64
            ^ demand
            ^ shared
            ^ u64::from(shifted)
            ^ known as u64
            ^ validity
            ^ snapshot
            ^ nested
            ^ recurrence
            ^ descending
            ^ wrapping;
    }
    println!("generic scalar: 20006 cases passed; checksum={checksum:016x}");
}
