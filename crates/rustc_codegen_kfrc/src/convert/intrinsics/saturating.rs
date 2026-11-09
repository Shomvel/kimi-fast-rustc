//! Saturating integer arithmetic in the original integer's exact bit width.

use super::*;

impl<'tcx> Converter<'tcx> {
    pub(super) fn intrinsic_saturating(
        &mut self,
        name: &str,
        ty: Ty<'tcx>,
        args: &[Operand],
    ) -> Result<Operand, String> {
        intrinsic_arity(name, args, 2)?;
        let (unsigned, bits) = self.intrinsic_unsigned(ty)?;
        let left = self.intrinsic_cast(args[0], unsigned);
        let right = self.intrinsic_cast(args[1], unsigned);
        let add = name == "saturating_add";
        let operation = if add { BinaryOp::Add } else { BinaryOp::Sub };
        // All generated arithmetic is wrapping unsigned arithmetic. In
        // particular, computing a saturated signed result never first invokes
        // an overflowing signed MIR operation.
        let result = self.intrinsic_temp(unsigned, Rvalue::BinaryOp(operation, left, right));
        let zero = self.intrinsic_constant(unsigned, 0);
        let (overflow, limit) = if matches!(ty.kind(), ty::Int(_)) {
            let sign_bit = 1u128 << (bits - 1);
            let result_sign_change =
                self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::BitXor, left, result));
            let other_sign_change = self.intrinsic_temp(
                unsigned,
                Rvalue::BinaryOp(BinaryOp::BitXor, right, if add { result } else { left }),
            );
            let sign_changes = self.intrinsic_temp(
                unsigned,
                Rvalue::BinaryOp(BinaryOp::BitAnd, result_sign_change, other_sign_change),
            );
            let sign_mask = self.intrinsic_constant(unsigned, sign_bit);
            let sign_changes = self.intrinsic_temp(
                unsigned,
                Rvalue::BinaryOp(BinaryOp::BitAnd, sign_changes, sign_mask),
            );
            let overflow = self.intrinsic_temp(
                self.tcx.types.bool,
                Rvalue::BinaryOp(BinaryOp::Ne, sign_changes, zero),
            );
            // Signed overflow clamps toward the sign of the left operand:
            // positive -> MAX; negative -> MIN, for both addition/subtraction.
            let sign_shift = self.intrinsic_constant(unsigned, u128::from(bits - 1));
            let sign =
                self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::Shr, left, sign_shift));
            let maximum = self.intrinsic_constant(unsigned, sign_bit - 1);
            let limit =
                self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::Add, maximum, sign));
            (overflow, limit)
        } else {
            let overflow = self.intrinsic_temp(
                self.tcx.types.bool,
                Rvalue::BinaryOp(
                    BinaryOp::Lt,
                    if add { result } else { left },
                    if add { left } else { right },
                ),
            );
            let limit =
                self.intrinsic_constant(unsigned, if add { (1u128 << bits) - 1 } else { 0 });
            (overflow, limit)
        };
        let overflow = self.intrinsic_cast(overflow, unsigned);
        let mask = self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::Sub, zero, overflow));
        let difference =
            self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::BitXor, result, limit));
        let correction = self.intrinsic_temp(
            unsigned,
            Rvalue::BinaryOp(BinaryOp::BitAnd, difference, mask),
        );
        let result = self.intrinsic_temp(
            unsigned,
            Rvalue::BinaryOp(BinaryOp::BitXor, result, correction),
        );
        Ok(self.intrinsic_cast(result, ty))
    }
}
