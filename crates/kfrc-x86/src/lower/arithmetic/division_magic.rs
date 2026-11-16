//! Reciprocal multiplication for unsigned division, using the same multiply-
//! high, optional add correction, and shift form as LLVM 21 DivisionByConstantInfo.
//! Unlike its APInt recurrence, u128 lets us directly compute the reciprocal.

pub(super) struct Magic {
    pub multiplier: u64,
    pub shift: u8,
    pub add: bool,
}

pub(super) fn unsigned(divisor: u64) -> Magic {
    debug_assert!(divisor > 1 && !divisor.is_power_of_two());
    let shift = (63 - divisor.leading_zeros()) as u8;
    let dividend = 1u128 << (64 + shift);
    let proposed = dividend / u128::from(divisor);
    let remainder = dividend % u128::from(divisor);
    // Round the reciprocal up. If the rounding error is too large at this
    // precision, use one extra bit and recover it as ((n - high) >> 1) + high.
    let add = u128::from(divisor) - remainder >= 1u128 << shift;
    let multiplier = if add {
        2 * proposed + 1 + u128::from(2 * remainder >= u128::from(divisor))
    } else {
        proposed + 1
    };
    Magic {
        multiplier: multiplier as u64,
        shift,
        add,
    }
}

#[test]
fn arbitrary_64_bit_divisors_match_integer_division() {
    let mut state = 0x2468ace013579bdfu64;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };
    for _ in 0..20_000 {
        let d = next().max(3);
        if d.is_power_of_two() {
            continue;
        }
        let magic = unsigned(d);
        for n in [
            0,
            1,
            d - 1,
            d,
            d.saturating_add(1),
            u64::MAX,
            next(),
            next(),
        ] {
            let high = ((u128::from(n) * u128::from(magic.multiplier)) >> 64) as u64;
            let corrected = if magic.add {
                ((n - high) >> 1) + high
            } else {
                high
            };
            assert_eq!(corrected >> magic.shift, n / d, "{n} / {d}");
        }
    }
}
