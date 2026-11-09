//! Select exact constant division without losing its interpreter-level contract.
use super::*;

impl Lowering<'_> {
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
