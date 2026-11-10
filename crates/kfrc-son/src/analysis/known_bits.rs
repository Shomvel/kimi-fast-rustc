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
        // Phi cycles, memory, arithmetic and provenance-sensitive values stay
        // unknown. No type-validity or non-poison claim is inferred here.
        _ => {}
    }
    debug_assert_eq!(result.zero & result.one, 0);
    Some(result)
}
