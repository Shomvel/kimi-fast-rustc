use super::{HoleAnswer, SpecializationOracle, State};
use crate::{ComparePredicate, NodeId, Opcode};
use kfrc_body::{TypeOracle, TypeShape};

impl<O: SpecializationOracle> State<'_, O> {
    pub(super) fn constant(&mut self, node: NodeId) -> Result<Option<u128>, String> {
        if let Some(value) = self.constants.get(&node) {
            return Ok(*value);
        }
        if !self.evaluating.insert(node) {
            return Ok(None);
        }
        self.stats.value_nodes_visited += 1;
        let result = self.evaluate_constant(node);
        self.evaluating.remove(&node);
        if let Ok(value) = result {
            self.constants.insert(node, value);
        }
        result
    }
    fn evaluate_constant(&mut self, node: NodeId) -> Result<Option<u128>, String> {
        let graph = self.source;
        let inputs = graph.inputs(node);
        match graph.op(node) {
            Opcode::Const => Ok(Some(graph.constant_value(node))),
            op if op.integer_intrinsic_arity().is_some() => {
                let Some(bits) = self
                    .bits(inputs[0])
                    .filter(|bits| matches!(bits, 8 | 16 | 32 | 64))
                else {
                    return Ok(None);
                };
                let ty = graph.rust_type(inputs[0]).expect("typed intrinsic");
                let signed = matches!(graph.oracle.shape(ty), TypeShape::Int { signed: true, .. });
                let mut values = Vec::with_capacity(inputs.len());
                for &input in inputs {
                    let Some(value) = self.constant(input)? else {
                        return Ok(None);
                    };
                    let Some(width) = self.bits(input).filter(|bits| *bits <= 64) else {
                        return Ok(None);
                    };
                    values.push((value & (u128::MAX >> (128 - width))) as u64);
                }
                let (value, poison) =
                    crate::intrinsic::evaluate(op, graph.payload(node), bits, signed, &values);
                Ok((!poison).then_some(u128::from(value)))
            }
            // A one-predecessor region adds no new choice. Do not speculate
            // through a multi-edge Phi: one arm may contain an unanswered,
            // ultimately dead question. Definition-level promotion produces
            // these one-edge Phis at ordinary MIR block boundaries.
            Opcode::Phi if inputs.len() == 2 => self.constant(inputs[1]),
            Opcode::Hole => match self.answer(graph.payload(node))? {
                HoleAnswer::NeedsDrop(value) => Ok(Some(u128::from(value))),
                _ => Ok(None),
            },
            Opcode::Projection if graph.op(inputs[0]) == Opcode::Hole => {
                match self.answer(graph.payload(inputs[0]))? {
                    HoleAnswer::SizeAlign(value) => {
                        Ok(Some(u128::from(match graph.payload(node) {
                            0 => value.size,
                            1 => value.align,
                            _ => return Err("SizeAlign projection is not size or alignment".into()),
                        })))
                    }
                    _ => Err("projected hole did not answer SizeAlign".into()),
                }
            }
            Opcode::BitNot | Opcode::Negate | Opcode::IntCast => {
                let Some(value) = self.constant(inputs[0])? else {
                    return Ok(None);
                };
                let Some(bits) = self.bits(node) else {
                    return Ok(None);
                };
                let mask = u128::MAX >> (128 - bits);
                Ok(Some(match graph.op(node) {
                    Opcode::BitNot => !value & mask,
                    Opcode::Negate => 0u128.wrapping_sub(value) & mask,
                    _ => {
                        let Some(source_bits) = self.bits(inputs[0]) else {
                            return Ok(None);
                        };
                        let signed = graph.rust_type(inputs[0]).is_some_and(|ty| {
                            matches!(graph.oracle.shape(ty), TypeShape::Int { signed: true, .. })
                        });
                        let value = if signed && bits > source_bits {
                            sign(value, source_bits) as u128
                        } else {
                            value
                        };
                        value & mask
                    }
                }))
            }
            Opcode::Compare => {
                let Some(left) = self.constant(inputs[0])? else {
                    return Ok(None);
                };
                let Some(right) = self.constant(inputs[1])? else {
                    return Ok(None);
                };
                let Some(bits) = self.bits(inputs[0]) else {
                    return Ok(None);
                };
                use ComparePredicate::*;
                let result = match ComparePredicate::from_payload(graph.payload(node)) {
                    Equal => left == right,
                    NotEqual => left != right,
                    SignedLess => sign(left, bits) < sign(right, bits),
                    SignedLessEqual => sign(left, bits) <= sign(right, bits),
                    SignedGreater => sign(left, bits) > sign(right, bits),
                    SignedGreaterEqual => sign(left, bits) >= sign(right, bits),
                    UnsignedLess => left < right,
                    UnsignedLessEqual => left <= right,
                    UnsignedGreater => left > right,
                    UnsignedGreaterEqual => left >= right,
                };
                Ok(Some(u128::from(result)))
            }
            Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::ShiftLeft
            | Opcode::ShiftRightArithmetic
            | Opcode::ShiftRightLogical => {
                let Some(left) = self.constant(inputs[0])? else {
                    return Ok(None);
                };
                let Some(right) = self.constant(inputs[1])? else {
                    return Ok(None);
                };
                let Some(bits) = self.bits(node) else {
                    return Ok(None);
                };
                let shift = (right % u128::from(bits)) as u32;
                let flags = crate::ArithmeticFlags(graph.payload(node));
                if flags.0 != 0
                    && (bits > 64
                        || flags.violated(graph.op(node), bits, left as u64, right as u64))
                {
                    return Ok(None);
                }
                let result = match graph.op(node) {
                    Opcode::Add => left.wrapping_add(right),
                    Opcode::Sub => left.wrapping_sub(right),
                    Opcode::Mul => left.wrapping_mul(right),
                    Opcode::BitAnd => left & right,
                    Opcode::BitOr => left | right,
                    Opcode::BitXor => left ^ right,
                    Opcode::ShiftLeft => left << shift,
                    Opcode::ShiftRightLogical => left >> shift,
                    Opcode::ShiftRightArithmetic => (sign(left, bits) >> shift) as u128,
                    _ => unreachable!(),
                };
                Ok(Some(result & (u128::MAX >> (128 - bits))))
            }
            _ => Ok(None),
        }
    }
    fn bits(&self, node: NodeId) -> Option<u8> {
        match self.source.oracle.shape(self.source.rust_type(node)?) {
            TypeShape::Bool => Some(1),
            TypeShape::Char => Some(32),
            TypeShape::Int { width, .. } => Some(width.bits(64)),
            _ => None,
        }
    }
}
fn sign(value: u128, bits: u8) -> i128 {
    ((value << (128 - bits)) as i128) >> (128 - bits)
}
