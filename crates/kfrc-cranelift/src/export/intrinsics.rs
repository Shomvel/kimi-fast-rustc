//! Integer intrinsics on Cranelift's bit-manipulation instructions.
//!
//! The target description is the SSE2 baseline, so Cranelift selects the
//! `bsf`/`bsr` and bit-twiddling sequences rather than POPCNT/LZCNT/TZCNT.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{InstBuilder, Value};
use kfrc_son::IntegerIntrinsicPayload as Payload;
use kfrc_son::{NodeId, Opcode};

use super::{Exporter, LowerError, Scalar};

impl Exporter<'_, '_> {
    pub(super) fn emit_integer_intrinsic(&mut self, node: NodeId) -> Result<Value, LowerError> {
        let graph = self.graph;
        let input = self.scalar_of_node(graph.input(node, 0))?;
        let ty = input.ty().ok_or_else(|| {
            LowerError::Malformed(format!("integer intrinsic {node} on a value without bits"))
        })?;
        let output = self.scalar_of_node(node)?;
        let payload = graph.payload(node);
        let value = match graph.op(node) {
            Opcode::BitCount => {
                let operand = self.operand(graph.input(node, 0), input.bits)?;
                // `clz` and `ctz` of zero are the bit width, the defined
                // result of the counts without ZERO_POISON.
                let count = match payload & 3 {
                    Payload::ONES => self.builder.ins().popcnt(operand),
                    Payload::LEADING_ZEROS => self.builder.ins().clz(operand),
                    Payload::TRAILING_ZEROS => self.builder.ins().ctz(operand),
                    kind => {
                        return Err(LowerError::Malformed(format!("bit count kind {kind}")));
                    }
                };
                let unsigned = Scalar {
                    bits: input.bits,
                    signed: false,
                };
                return Ok(self.coerce(count, unsigned, output.bits));
            }
            Opcode::ByteSwap => {
                let operand = self.operand(graph.input(node, 0), input.bits)?;
                if input.bits == 8 {
                    operand
                } else {
                    self.builder.ins().bswap(operand)
                }
            }
            Opcode::BitReverse => {
                let operand = self.operand(graph.input(node, 0), input.bits)?;
                self.builder.ins().bitrev(operand)
            }
            Opcode::FunnelShift => {
                let right = payload & Payload::FUNNEL_RIGHT != 0;
                let high_node = graph.resolve(graph.input(node, 0));
                let low_node = graph.resolve(graph.input(node, 1));
                let high = self.operand(high_node, input.bits)?;
                let amount = self.value(graph.input(node, 2))?;
                if high_node == low_node {
                    if right {
                        self.builder.ins().rotr(high, amount)
                    } else {
                        self.builder.ins().rotl(high, amount)
                    }
                } else {
                    let low = self.operand(low_node, input.bits)?;
                    // With s = amount mod width: left is
                    // (high << s) | (low >> (width - s)), right is
                    // (high << (width - s)) | (low >> s). The complementary
                    // shift is split in two so that s = 0 contributes zero.
                    let amount_scalar = self.scalar_of_node(graph.input(node, 2))?;
                    let amount = self.coerce(amount, amount_scalar, input.bits);
                    let mask = self
                        .builder
                        .ins()
                        .band_imm_u(amount, i64::from(input.bits) - 1);
                    let last = self.integer_constant(ty, u64::from(input.bits) - 1);
                    let complement = self.builder.ins().isub(last, mask);
                    if right {
                        let primary = self.builder.ins().ushr(low, mask);
                        let partial = self.builder.ins().ishl_imm_u(high, 1);
                        let secondary = self.builder.ins().ishl(partial, complement);
                        self.builder.ins().bor(primary, secondary)
                    } else {
                        let primary = self.builder.ins().ishl(high, mask);
                        let partial = self.builder.ins().ushr_imm_u(low, 1);
                        let secondary = self.builder.ins().ushr(partial, complement);
                        self.builder.ins().bor(primary, secondary)
                    }
                }
            }
            op @ (Opcode::SaturatingAdd | Opcode::SaturatingSub) => {
                let subtract = op == Opcode::SaturatingSub;
                let lhs = self.operand(graph.input(node, 0), input.bits)?;
                let rhs = self.operand(graph.input(node, 1), input.bits)?;
                if input.signed {
                    let (result, overflow) = if subtract {
                        self.builder.ins().ssub_overflow(lhs, rhs)
                    } else {
                        self.builder.ins().sadd_overflow(lhs, rhs)
                    };
                    // On overflow the result has the sign opposite to the
                    // mathematical value: saturate towards the sign of `lhs`.
                    let sign = self
                        .builder
                        .ins()
                        .sshr_imm_u(lhs, i64::from(input.bits) - 1);
                    let maximum = (1u64 << (input.bits - 1)) - 1;
                    let maximum = self.integer_constant(ty, maximum);
                    let limit = self.builder.ins().bxor(sign, maximum);
                    self.builder.ins().select(overflow, limit, result)
                } else if subtract {
                    let difference = self.builder.ins().isub(lhs, rhs);
                    let overflow = self
                        .builder
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, lhs, rhs);
                    let zero = self.builder.ins().iconst(ty, 0);
                    self.builder.ins().select(overflow, zero, difference)
                } else {
                    let sum = self.builder.ins().iadd(lhs, rhs);
                    let overflow = self.builder.ins().icmp(IntCC::UnsignedLessThan, sum, lhs);
                    let maximum = self.integer_constant(ty, u64::MAX);
                    self.builder.ins().select(overflow, maximum, sum)
                }
            }
            other => {
                return Err(LowerError::Malformed(format!(
                    "{} is not an integer intrinsic",
                    other.name()
                )));
            }
        };
        Ok(self.coerce(value, input, output.bits))
    }
}
