//! Target-independent integer intrinsic contracts, kept through R/S/L.
use kfrc_body::{BitCountKind, FunnelDirection, IntegerIntrinsic};

use crate::Opcode;

/// Payload bits are opcode-local; they are not ArithmeticFlags.
pub struct IntegerIntrinsicPayload;
impl IntegerIntrinsicPayload {
    pub const ONES: u32 = 0;
    pub const LEADING_ZEROS: u32 = 1;
    pub const TRAILING_ZEROS: u32 = 2;
    pub const ZERO_POISON: u32 = 4;
    pub const FUNNEL_RIGHT: u32 = 1;
    pub const AMOUNT_IN_RANGE: u32 = 2;

    pub fn from_body(op: &IntegerIntrinsic) -> (Opcode, u32) {
        match *op {
            IntegerIntrinsic::BitCount { kind, zero_poison } => (
                Opcode::BitCount,
                match kind {
                    BitCountKind::Ones => Self::ONES,
                    BitCountKind::LeadingZeros => Self::LEADING_ZEROS,
                    BitCountKind::TrailingZeros => Self::TRAILING_ZEROS,
                } | if zero_poison { Self::ZERO_POISON } else { 0 },
            ),
            IntegerIntrinsic::ByteSwap => (Opcode::ByteSwap, 0),
            IntegerIntrinsic::BitReverse => (Opcode::BitReverse, 0),
            IntegerIntrinsic::FunnelShift {
                direction,
                amount_in_range,
            } => (
                Opcode::FunnelShift,
                if direction == FunnelDirection::Right {
                    Self::FUNNEL_RIGHT
                } else {
                    0
                } | if amount_in_range {
                    Self::AMOUNT_IN_RANGE
                } else {
                    0
                },
            ),
            IntegerIntrinsic::SaturatingAdd => (Opcode::SaturatingAdd, 0),
            IntegerIntrinsic::SaturatingSub => (Opcode::SaturatingSub, 0),
        }
    }

    pub fn valid(op: Opcode, payload: u32) -> bool {
        match op {
            Opcode::BitCount => matches!(payload, 0 | 1 | 2 | 5 | 6),
            Opcode::FunnelShift => payload & !3 == 0,
            Opcode::ByteSwap
            | Opcode::BitReverse
            | Opcode::SaturatingAdd
            | Opcode::SaturatingSub => payload == 0,
            _ => false,
        }
    }

    pub fn may_produce_poison(op: Opcode, payload: u32) -> bool {
        match op {
            Opcode::BitCount => payload & Self::ZERO_POISON != 0,
            Opcode::FunnelShift => payload & Self::AMOUNT_IN_RANGE != 0,
            _ => false,
        }
    }
}

impl Opcode {
    pub fn integer_intrinsic_arity(self) -> Option<usize> {
        match self {
            Self::BitCount | Self::ByteSwap | Self::BitReverse => Some(1),
            Self::FunnelShift => Some(3),
            Self::SaturatingAdd | Self::SaturatingSub => Some(2),
            _ => None,
        }
    }
}

/// Returns canonical low bits and contract poison, independently of input poison.
/// Callers have verified operand widths and payloads; unsupported widths never
/// enter this <=64-bit execution path.
pub(crate) fn evaluate(
    op: Opcode,
    payload: u32,
    width: u8,
    signed: bool,
    values: &[u64],
) -> (u64, bool) {
    use IntegerIntrinsicPayload as P;
    debug_assert!(matches!(width, 8 | 16 | 32 | 64));
    let mask = u64::MAX >> (64 - width);
    let a = values[0] & mask;
    let b = values.get(1).copied().unwrap_or(0) & mask;
    let sign = |value: u64| i128::from(((value << (64 - width)) as i64) >> (64 - width));
    let result = match op {
        Opcode::BitCount => {
            let count = match payload & 3 {
                P::ONES => a.count_ones(),
                P::LEADING_ZEROS => a.leading_zeros() - (64 - u32::from(width)),
                P::TRAILING_ZEROS => a.trailing_zeros().min(u32::from(width)),
                _ => unreachable!("verified bit count kind"),
            };
            return (u64::from(count), a == 0 && payload & P::ZERO_POISON != 0);
        }
        Opcode::ByteSwap => a.swap_bytes() >> (64 - width),
        Opcode::BitReverse => a.reverse_bits() >> (64 - width),
        Opcode::FunnelShift => {
            let raw = values[2];
            let shift = (raw % u64::from(width)) as u32;
            let right = payload & P::FUNNEL_RIGHT != 0;
            let bits = if shift == 0 {
                if right { b } else { a }
            } else if right {
                (a << (u32::from(width) - shift)) | (b >> shift)
            } else {
                (a << shift) | (b >> (u32::from(width) - shift))
            };
            return (
                bits & mask,
                payload & P::AMOUNT_IN_RANGE != 0 && raw >= u64::from(width),
            );
        }
        Opcode::SaturatingAdd | Opcode::SaturatingSub => {
            if signed {
                let value = if op == Opcode::SaturatingAdd {
                    sign(a) + sign(b)
                } else {
                    sign(a) - sign(b)
                };
                let lo = -(1i128 << (width - 1));
                let hi = (1i128 << (width - 1)) - 1;
                value.clamp(lo, hi) as u64
            } else if op == Opcode::SaturatingAdd {
                (u128::from(a) + u128::from(b)).min(u128::from(mask)) as u64
            } else {
                a.saturating_sub(b)
            }
        }
        _ => unreachable!("integer intrinsic"),
    };
    (result & mask, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_saturating_pair_and_funnel_pair_matches_integer_arithmetic() {
        for a in 0u16..=255 {
            for b in 0u16..=255 {
                for signed in [false, true] {
                    let (x, y) = if signed {
                        (i32::from(a as i8), i32::from(b as i8))
                    } else {
                        (i32::from(a), i32::from(b))
                    };
                    let (lo, hi) = if signed { (-128, 127) } else { (0, 255) };
                    for (op, mathematical) in [
                        (Opcode::SaturatingAdd, x + y),
                        (Opcode::SaturatingSub, x - y),
                    ] {
                        assert_eq!(
                            evaluate(op, 0, 8, signed, &[u64::from(a), u64::from(b)]),
                            (u64::from(mathematical.clamp(lo, hi) as u8), false)
                        );
                    }
                }
                let joined = (u32::from(a) << 8) | u32::from(b);
                for amount in 0..8 {
                    let args = [u64::from(a), u64::from(b), amount];
                    assert_eq!(
                        evaluate(Opcode::FunnelShift, 0, 8, false, &args),
                        (u64::from(((joined << amount) >> 8) & 255), false)
                    );
                    assert_eq!(
                        evaluate(Opcode::FunnelShift, 1, 8, false, &args),
                        (u64::from((joined >> amount) & 255), false)
                    );
                }
            }
        }
    }
}
