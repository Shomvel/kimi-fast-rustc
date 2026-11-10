//! Select constant division while preserving interpreter-level contracts.
use super::*;

mod division_magic;

impl Lowering<'_> {
    pub(super) fn emit_constant_division(
        &mut self,
        node: NodeId,
        lhs: VirtualValue,
        rhs: VirtualValue,
    ) -> Result<bool, LowerError> {
        let VirtualValue::Imm(divisor) = rhs else {
            return Ok(false);
        };
        let op = self.graph.op(node);
        let signed = matches!(op, Opcode::DivSigned | Opcode::RemSigned);
        // Retain hardware traps for zero and MIN/-1. Contract-bearing nodes
        // are handled separately; this expansion stays at the fixed node's
        // schedule point and does not change the S/L poison observations.
        if self.graph.payload(node) != 0 || divisor == 0 || (signed && divisor < 0) {
            return Ok(false);
        }
        let kind = self.node_kind(node)?;
        let divisor = divisor as u64;
        let remainder = matches!(op, Opcode::RemSigned | Opcode::RemUnsigned);
        let key = (self.graph.input(node, 0), divisor, signed);
        if let Some(&quotient) = self.constant_quotients.get(&key) {
            self.finish_constant_division(node, lhs, quotient, divisor, remainder, kind);
            return Ok(true);
        }
        let mut quotient = lhs;
        if divisor.is_power_of_two() {
            let shift = divisor.trailing_zeros();
            if !signed && remainder {
                let result = self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::And { dst, lhs, rhs },
                    lhs,
                    VirtualValue::Imm((divisor - 1) as i64),
                );
                self.canonicalize(register(node), result, kind);
                return Ok(true);
            }
            if shift != 0 {
                if signed {
                    let sign = self.arithmetic_binary(
                        |dst, lhs, rhs| VirtualInstr::Shr { dst, lhs, rhs },
                        lhs,
                        VirtualValue::Imm(63),
                    );
                    let bias = self.arithmetic_binary(
                        |dst, lhs, rhs| VirtualInstr::LShr { dst, lhs, rhs },
                        sign,
                        VirtualValue::Imm(i64::from(64 - shift)),
                    );
                    let biased = self.arithmetic_binary(
                        |dst, lhs, rhs| VirtualInstr::Add { dst, lhs, rhs },
                        lhs,
                        bias,
                    );
                    quotient = self.arithmetic_binary(
                        |dst, lhs, rhs| VirtualInstr::Shr { dst, lhs, rhs },
                        biased,
                        VirtualValue::Imm(i64::from(shift)),
                    );
                } else {
                    quotient = self.arithmetic_binary(
                        |dst, lhs, rhs| VirtualInstr::LShr { dst, lhs, rhs },
                        lhs,
                        VirtualValue::Imm(i64::from(shift)),
                    );
                }
            }
        } else {
            let sign = if signed {
                Some(self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::Shr { dst, lhs, rhs },
                    lhs,
                    VirtualValue::Imm(63),
                ))
            } else {
                None
            };
            let magnitude = if let Some(sign) = sign {
                let flipped = self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::Xor { dst, lhs, rhs },
                    lhs,
                    sign,
                );
                self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::Sub { dst, lhs, rhs },
                    flipped,
                    sign,
                )
            } else {
                lhs
            };
            let magic = division_magic::unsigned(divisor);
            quotient = self.arithmetic_binary(
                |dst, lhs, rhs| VirtualInstr::MulHigh { dst, lhs, rhs },
                magnitude,
                VirtualValue::Imm(magic.multiplier as i64),
            );
            if magic.add {
                let difference = self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::Sub { dst, lhs, rhs },
                    magnitude,
                    quotient,
                );
                let half = self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::LShr { dst, lhs, rhs },
                    difference,
                    VirtualValue::Imm(1),
                );
                quotient = self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::Add { dst, lhs, rhs },
                    quotient,
                    half,
                );
            }
            if magic.shift != 0 {
                quotient = self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::LShr { dst, lhs, rhs },
                    quotient,
                    VirtualValue::Imm(i64::from(magic.shift)),
                );
            }
            if let Some(sign) = sign {
                let flipped = self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::Xor { dst, lhs, rhs },
                    quotient,
                    sign,
                );
                quotient = self.arithmetic_binary(
                    |dst, lhs, rhs| VirtualInstr::Sub { dst, lhs, rhs },
                    flipped,
                    sign,
                );
            }
        }
        self.constant_quotients.insert(key, quotient);
        self.finish_constant_division(node, lhs, quotient, divisor, remainder, kind);
        Ok(true)
    }

    fn finish_constant_division(
        &mut self,
        node: NodeId,
        lhs: VirtualValue,
        quotient: VirtualValue,
        divisor: u64,
        remainder: bool,
        kind: ScalarKind,
    ) {
        let result = if remainder {
            let product = self.arithmetic_binary(
                |dst, lhs, rhs| VirtualInstr::Mul { dst, lhs, rhs },
                quotient,
                VirtualValue::Imm(divisor as i64),
            );
            self.arithmetic_binary(
                |dst, lhs, rhs| VirtualInstr::Sub { dst, lhs, rhs },
                lhs,
                product,
            )
        } else {
            quotient
        };
        self.canonicalize(register(node), result, kind);
    }

    fn arithmetic_binary(
        &mut self,
        make: impl FnOnce(VirtualReg, VirtualValue, VirtualValue) -> VirtualInstr,
        lhs: VirtualValue,
        rhs: VirtualValue,
    ) -> VirtualValue {
        let dst = self.temporary();
        self.push(make(dst, lhs, rhs));
        VirtualValue::Reg(dst)
    }

    pub(super) fn emit_exact_constant_division(
        &mut self,
        node: NodeId,
        lhs: VirtualValue,
        rhs: VirtualValue,
    ) -> Result<bool, LowerError> {
        let op = self.graph.op(node);
        if !matches!(op, Opcode::DivSigned | Opcode::DivUnsigned)
            || !ArithmeticFlags(self.graph.payload(node)).contains(ArithmeticFlags::EXACT)
        {
            return Ok(false);
        }
        let VirtualValue::Imm(divisor) = rhs else {
            return Ok(false);
        };
        let kind = self.node_kind(node)?;
        if divisor == 0 || (kind.signed && divisor < 0) {
            return Ok(false);
        }
        let divisor = divisor as u64;
        let shift = divisor.trailing_zeros();
        let odd = divisor >> shift;
        let mut value = lhs;
        if shift != 0 {
            let shifted = self.temporary();
            self.push(if kind.signed {
                VirtualInstr::Shr {
                    dst: shifted,
                    lhs: value,
                    rhs: VirtualValue::Imm(i64::from(shift)),
                }
            } else {
                VirtualInstr::LShr {
                    dst: shifted,
                    lhs: value,
                    rhs: VirtualValue::Imm(i64::from(shift)),
                }
            });
            value = VirtualValue::Reg(shifted);
        }
        if odd != 1 {
            // Odd d has an inverse modulo 2^64. Each Newton step doubles
            // correct low bits; exact division makes multiplication by this
            // inverse equal to the mathematical quotient at the IR width.
            let mut inverse = 1u64;
            for _ in 0..6 {
                inverse = inverse.wrapping_mul(2u64.wrapping_sub(odd.wrapping_mul(inverse)));
            }
            let product = self.temporary();
            self.push(VirtualInstr::Mul {
                dst: product,
                lhs: value,
                rhs: VirtualValue::Imm(inverse as i64),
            });
            value = VirtualValue::Reg(product);
        }
        self.canonicalize(register(node), value, kind);
        Ok(true)
    }
}
