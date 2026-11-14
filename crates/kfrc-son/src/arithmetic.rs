//! Poison-producing integer contracts carried in an arithmetic node's payload.
//!
//! These are promises about the source operation, not inferred machine-register
//! properties. Rewrites must prove a promise again before attaching it to a new
//! operation. Ordinary wrapping arithmetic carries no flags.
use crate::Opcode;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArithmeticFlags(pub u32);

impl ArithmeticFlags {
    pub const NUW: u32 = 1 << 0;
    pub const NSW: u32 = 1 << 1;
    pub const EXACT: u32 = 1 << 2;
    pub const DISJOINT: u32 = 1 << 3;
    pub const AMOUNT_IN_RANGE: u32 = 1 << 4;

    pub fn contains(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    /// `None` means that the opcode's payload has a different meaning.
    pub fn allowed(op: Opcode) -> Option<u32> {
        use Opcode::*;
        Some(match op {
            Add | Sub | Mul => Self::NUW | Self::NSW,
            ShiftLeft => Self::NUW | Self::NSW | Self::AMOUNT_IN_RANGE,
            ShiftRightArithmetic | ShiftRightLogical => Self::EXACT | Self::AMOUNT_IN_RANGE,
            DivSigned | DivUnsigned => Self::EXACT,
            BitOr => Self::DISJOINT,
            RemSigned | RemUnsigned | BitAnd | BitXor | Negate | BitNot | IntCast | MulOverflow => {
                0
            }
            _ => return None,
        })
    }

    pub fn valid_for(self, op: Opcode) -> bool {
        Self::allowed(op).is_some_and(|allowed| self.0 & !allowed == 0)
    }

    /// Interpret the contract at the IR operand width (the PoC supports <=64).
    /// Division's zero/overflow traps remain the interpreter's responsibility.
    pub fn violated(self, op: Opcode, width: u8, lhs: u64, rhs: u64) -> bool {
        if self.0 == 0 {
            return false;
        }
        debug_assert!((1..=64).contains(&width));
        let mask = u64::MAX >> (64 - width);
        let a = lhs & mask;
        let b = rhs & mask;
        let signed = |value: u64| {
            let shift = 64 - width;
            i128::from(((value << shift) as i64) >> shift)
        };
        let sa = signed(a);
        let sb = signed(b);
        let amount = (rhs % u64::from(width)) as u32;
        if self.contains(Self::AMOUNT_IN_RANGE) && rhs >= u64::from(width) {
            return true;
        }
        if self.contains(Self::DISJOINT) && a & b != 0 {
            return true;
        }
        if self.contains(Self::EXACT) {
            let inexact = match op {
                Opcode::DivUnsigned => b != 0 && a % b != 0,
                Opcode::DivSigned => sb != 0 && sa % sb != 0,
                Opcode::ShiftRightArithmetic | Opcode::ShiftRightLogical => {
                    a & ((1u64 << amount) - 1) != 0
                }
                _ => false,
            };
            if inexact {
                return true;
            }
        }
        if self.contains(Self::NUW) {
            let overflow = match op {
                Opcode::Add => u128::from(a) + u128::from(b) > u128::from(mask),
                Opcode::Sub => a < b,
                Opcode::Mul => u128::from(a) * u128::from(b) > u128::from(mask),
                Opcode::ShiftLeft => u128::from(a) << amount > u128::from(mask),
                _ => false,
            };
            if overflow {
                return true;
            }
        }
        if self.contains(Self::NSW) {
            let value = match op {
                Opcode::Add => sa + sb,
                Opcode::Sub => sa - sb,
                Opcode::Mul => sa * sb,
                Opcode::ShiftLeft => sa << amount,
                _ => 0,
            };
            let minimum = -(1i128 << (width - 1));
            let maximum = (1i128 << (width - 1)) - 1;
            if value < minimum || value > maximum {
                return true;
            }
        }
        false
    }
}
