//! Bits fixed whenever a scalar value is defined. This is not a proof that the
//! value is non-poison: callers must retain its observation when rewriting.
//! Queries are bounded and local, so no facts survive graph mutation.

use kfrc_body::{TypeOracle, TypeShape};

use crate::{FunctionGraph, NodeId, Opcode};

#[derive(Clone, Copy)]
pub(crate) struct KnownBits {
    pub zero: u64,
    pub one: u64,
    bits: u8,
}

impl KnownBits {
    pub(crate) fn unsigned_bounds(self) -> (u64, u64) {
        (self.one, self.mask() & !self.zero)
    }
    /// Whether a closed unsigned interval contains a value with these bits.
    /// Unlike the convex bounds, this preserves holes such as {0, 2}.
    pub(crate) fn intersects_unsigned_range(self, min: u64, max: u64) -> bool {
        let max = max.min(self.mask());
        if min > max {
            return false;
        }
        let mut candidate = (min | self.one) & !self.zero & self.mask();
        if candidate > min {
            // A required one already raised the prefix above `min`; the
            // lower variable bits must be cleared to get its least successor.
            let mismatch = 63 - (candidate ^ min).leading_zeros();
            candidate = (candidate & !((1u64 << mismatch) - 1)) | self.one;
        } else if candidate < min {
            // The highest mismatch cleared a forbidden one in `min`. Round
            // up at the next available higher zero, then minimize its suffix.
            let mismatch = 63 - (candidate ^ min).leading_zeros();
            let higher = u64::MAX.checked_shl(mismatch + 1).unwrap_or(0);
            let available = !min & !self.zero & self.mask() & higher;
            if available == 0 {
                return false;
            }
            let increment = 1u64 << available.trailing_zeros();
            candidate = (min & !(increment | (increment - 1))) | increment | self.one;
        }
        candidate <= max
    }
    fn mask(self) -> u64 {
        u64::MAX >> (64 - self.bits)
    }
    fn unknown(bits: u8) -> Self {
        Self {
            zero: 0,
            one: 0,
            bits,
        }
    }
}

fn kind(g: &FunctionGraph, node: NodeId) -> Option<(u8, bool)> {
    match g.oracle.shape(g.rust_type(node)?) {
        TypeShape::Bool => Some((1, false)),
        TypeShape::Int { width, signed } if width.bits(64) <= 64 => Some((width.bits(64), *signed)),
        _ => None,
    }
}

pub(crate) fn of(graph: &FunctionGraph, node: NodeId) -> Option<KnownBits> {
    query(graph, node, &mut 32)
}

fn query(g: &FunctionGraph, node: NodeId, remaining: &mut usize) -> Option<KnownBits> {
    let node = g.resolve(node);
    let (bits, _) = kind(g, node)?;
    let mut result = KnownBits::unknown(bits);
    if *remaining == 0 {
        return Some(result);
    }
    *remaining -= 1;
    let mask = result.mask();
    match g.op(node) {
        Opcode::Const => {
            result.one = g.constant_value(node) as u64 & mask;
            result.zero = !result.one & mask;
        }
        Opcode::BitAnd | Opcode::BitOr | Opcode::BitXor => {
            let a = query(g, g.input(node, 0), remaining)?;
            let b = query(g, g.input(node, 1), remaining)?;
            match g.op(node) {
                Opcode::BitAnd => {
                    result.zero = a.zero | b.zero;
                    result.one = a.one & b.one;
                }
                Opcode::BitOr => {
                    result.zero = a.zero & b.zero;
                    result.one = a.one | b.one;
                }
                Opcode::BitXor => {
                    result.zero = (a.zero & b.zero) | (a.one & b.one);
                    result.one = (a.zero & b.one) | (a.one & b.zero);
                }
                _ => unreachable!(),
            }
        }
        Opcode::BitNot => {
            let a = query(g, g.input(node, 0), remaining)?;
            result.zero = a.one;
            result.one = a.zero;
        }
        Opcode::Sub => {
            // (x + y) - x has y's bits whenever the expression is defined,
            // including modular arithmetic. This does not erase poison in S.
            let sum = g.resolve(g.input(node, 0));
            let subtrahend = g.resolve(g.input(node, 1));
            if g.op(sum) == Opcode::Add {
                let a = g.resolve(g.input(sum, 0));
                let b = g.resolve(g.input(sum, 1));
                if a == subtrahend {
                    result = query(g, b, remaining)?;
                } else if b == subtrahend {
                    result = query(g, a, remaining)?;
                }
            }
        }
        Opcode::Phi => {
            // Undefined incoming values contribute no defined executions.
            // Intersect every other arm; recursive queries exhaust the shared
            // budget to unknown, so loops never bootstrap facts from themselves.
            let mut common: Option<KnownBits> = None;
            for &incoming in &g.inputs(node)[1..] {
                if g.op(g.resolve(incoming)) == Opcode::Undefined {
                    continue;
                }
                let bits = query(g, incoming, remaining)?;
                common = Some(match common {
                    None => bits,
                    Some(old) => KnownBits {
                        zero: old.zero & bits.zero,
                        one: old.one & bits.one,
                        bits: old.bits,
                    },
                });
            }
            if let Some(common) = common {
                result = common;
            }
        }
        Opcode::IntCast => {
            let input = g.input(node, 0);
            let a = query(g, input, remaining)?;
            let (_, signed) = kind(g, input)?;
            result.zero = a.zero & mask;
            result.one = a.one & mask;
            if bits > a.bits {
                let high = mask & !a.mask();
                let sign = 1u64 << (a.bits - 1);
                if !signed || a.zero & sign != 0 {
                    result.zero |= high;
                }
                if signed && a.one & sign != 0 {
                    result.one |= high;
                }
            }
        }
        Opcode::ShiftLeft | Opcode::ShiftRightLogical | Opcode::ShiftRightArithmetic => {
            let shift = g.resolve(g.input(node, 1));
            if g.op(shift) != Opcode::Const {
                return Some(result);
            }
            let amount = (g.constant_value(shift) % u128::from(bits)) as u32;
            let a = query(g, g.input(node, 0), remaining)?;
            let low = (1u64 << amount) - 1;
            match g.op(node) {
                Opcode::ShiftLeft => {
                    result.zero = ((a.zero << amount) | low) & mask;
                    result.one = (a.one << amount) & mask;
                }
                Opcode::ShiftRightLogical | Opcode::ShiftRightArithmetic => {
                    let high = mask ^ (mask >> amount);
                    result.zero = a.zero >> amount;
                    result.one = a.one >> amount;
                    let sign = 1u64 << (bits - 1);
                    if g.op(node) == Opcode::ShiftRightLogical || a.zero & sign != 0 {
                        result.zero |= high;
                    }
                    if g.op(node) == Opcode::ShiftRightArithmetic && a.one & sign != 0 {
                        result.one |= high;
                    }
                }
                _ => unreachable!(),
            }
        }
        // Memory, other arithmetic and provenance-sensitive values stay
        // unknown. No type-validity or non-poison claim is inferred here.
        _ => {}
    }
    debug_assert_eq!(result.zero & result.one, 0);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValueType;
    use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign};

    #[test]
    fn interval_intersection_matches_every_byte_mask() {
        for zero in 0..=255u64 {
            for one in 0..=255u64 {
                if zero & one != 0 {
                    continue;
                }
                let facts = KnownBits { zero, one, bits: 8 };
                let mut next = None;
                for min in (0..=255u64).rev() {
                    if min & zero == 0 && min & one == one {
                        next = Some(min);
                    }
                    for max in [min, (min + 1).min(255), (min + 31).min(255), 255, u64::MAX] {
                        assert_eq!(
                            facts.intersects_unsigned_range(min, max),
                            next.is_some_and(|value| value <= max),
                            "zero={zero:#x} one={one:#x} min={min} max={max}"
                        );
                    }
                }
                assert!(!facts.intersects_unsigned_range(256, u64::MAX));
                assert!(!facts.intersects_unsigned_range(1, 0));
            }
        }
        // Exercise the top-bit mismatch without shifting by the word width.
        let low_half = KnownBits {
            zero: 1 << 63,
            one: 0,
            bits: 64,
        };
        assert!(!low_half.intersects_unsigned_range(1 << 63, u64::MAX));
        let high_half = KnownBits {
            zero: 0,
            one: 1 << 63,
            bits: 64,
        };
        assert!(high_half.intersects_unsigned_range(0, 1 << 63));
        assert!(!high_half.intersects_unsigned_range(0, (1 << 63) - 1));
    }

    #[test]
    fn a_cycle_cannot_supply_its_own_known_bits() {
        let mut types = RecordedTypes::new();
        let ty = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let mut graph = FunctionGraph::new(
            "cyclic_facts",
            FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
            types,
        );
        let region = graph.add_node(Opcode::Region, NodeId::NONE, &[], 0, ValueType::Control);
        let one = graph.add_constant(1, ty);
        let phi = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, one, one],
            0,
            ValueType::Rust(ty),
        );
        graph.set_input(phi, 2, phi);
        assert_eq!(of(&graph, phi).unwrap().unsigned_bounds(), (0, 255));
        let zero = graph.add_constant(0, ty);
        graph.set_input(phi, 2, zero);
        assert_eq!(of(&graph, phi).unwrap().unsigned_bounds(), (0, 1));
    }
}
