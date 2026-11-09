//! Integer bit operations expressed without target-specific intrinsics.

use super::*;

impl<'tcx> Converter<'tcx> {
    pub(super) fn intrinsic_funnel(
        &mut self,
        name: &str,
        ty: Ty<'tcx>,
        args: &[Operand],
    ) -> Result<Operand, String> {
        let rotate = matches!(name, "rotate_left" | "rotate_right");
        intrinsic_arity(name, args, if rotate { 2 } else { 3 })?;
        let (unsigned, bits) = self.intrinsic_unsigned(ty)?;
        let left = self.intrinsic_cast(args[0], unsigned);
        let right = if rotate {
            left
        } else {
            self.intrinsic_cast(args[1], unsigned)
        };
        let amount = self.intrinsic_cast(args[if rotate { 1 } else { 2 }], unsigned);
        let mask = self.intrinsic_constant(unsigned, u128::from(bits - 1));
        // Safe rotate accepts every u32 count. Funnel's unchecked contract already
        // requires count < bits; masking also keeps every generated shift in range.
        let amount =
            self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::BitAnd, amount, mask));
        let zero = self.intrinsic_constant(unsigned, 0);
        let complement =
            self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::Sub, zero, amount));
        let complement = self.intrinsic_temp(
            unsigned,
            Rvalue::BinaryOp(BinaryOp::BitAnd, complement, mask),
        );
        let is_left = matches!(name, "rotate_left" | "unchecked_funnel_shl");
        let (primary, secondary, primary_op, secondary_op) = if is_left {
            (left, right, BinaryOp::Shl, BinaryOp::Shr)
        } else {
            (right, left, BinaryOp::Shr, BinaryOp::Shl)
        };
        let primary = self.intrinsic_temp(unsigned, Rvalue::BinaryOp(primary_op, primary, amount));
        let secondary = self.intrinsic_temp(
            unsigned,
            Rvalue::BinaryOp(secondary_op, secondary, complement),
        );
        // A masked shift by `bits - 0` becomes a shift by zero. Suppress that
        // secondary half: fshl(a,b,0) is a, while fshr(a,b,0) is b, even a != b.
        let nonzero = self.intrinsic_temp(
            self.tcx.types.bool,
            Rvalue::BinaryOp(BinaryOp::Ne, amount, zero),
        );
        let nonzero = self.intrinsic_cast(nonzero, unsigned);
        let secondary_mask =
            self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::Sub, zero, nonzero));
        let secondary = self.intrinsic_temp(
            unsigned,
            Rvalue::BinaryOp(BinaryOp::BitAnd, secondary, secondary_mask),
        );
        let result = self.intrinsic_temp(
            unsigned,
            Rvalue::BinaryOp(BinaryOp::BitOr, primary, secondary),
        );
        Ok(self.intrinsic_cast(result, ty))
    }

    pub(super) fn intrinsic_bits(
        &mut self,
        name: &str,
        ty: Ty<'tcx>,
        args: &[Operand],
    ) -> Result<Operand, String> {
        intrinsic_arity(name, args, 1)?;
        let (unsigned, bits) = self.intrinsic_unsigned(ty)?;
        let input = self.intrinsic_cast(args[0], unsigned);
        let result = if matches!(name, "bswap" | "bitreverse") {
            let unit = if name == "bswap" { 8 } else { 1 };
            let mask = self.intrinsic_constant(unsigned, (1u128 << unit) - 1);
            let mut result = self.intrinsic_constant(unsigned, 0);
            for index in 0..bits / unit {
                let from = self.intrinsic_constant(unsigned, u128::from(index * unit));
                let shifted =
                    self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::Shr, input, from));
                let masked = self
                    .intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::BitAnd, shifted, mask));
                let to = self.intrinsic_constant(unsigned, u128::from(bits - (index + 1) * unit));
                let part =
                    self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::Shl, masked, to));
                result =
                    self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::BitOr, result, part));
            }
            result
        } else {
            let result_ty = self.tcx.types.u32;
            let zero = self.intrinsic_constant(unsigned, 0);
            let one = self.intrinsic_constant(unsigned, 1);
            let mut count = self.intrinsic_constant(result_ty, 0);
            let mut prefix = self.intrinsic_constant(self.tcx.types.bool, 1);
            for index in 0..bits {
                let index = if name.starts_with("ctlz") {
                    bits - index - 1
                } else {
                    index
                };
                let amount = self.intrinsic_constant(unsigned, u128::from(index));
                let shifted =
                    self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::Shr, input, amount));
                let bit =
                    self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::BitAnd, shifted, one));
                let increment = if name == "ctpop" {
                    self.intrinsic_cast(bit, result_ty)
                } else {
                    let clear = self.intrinsic_temp(
                        self.tcx.types.bool,
                        Rvalue::BinaryOp(BinaryOp::Eq, bit, zero),
                    );
                    prefix = self.intrinsic_temp(
                        self.tcx.types.bool,
                        Rvalue::BinaryOp(BinaryOp::BitAnd, prefix, clear),
                    );
                    self.intrinsic_cast(prefix, result_ty)
                };
                count = self
                    .intrinsic_temp(result_ty, Rvalue::BinaryOp(BinaryOp::Add, count, increment));
            }
            count
        };
        let result = if matches!(name, "bswap" | "bitreverse") {
            self.intrinsic_cast(result, ty)
        } else {
            result
        };
        Ok(result)
    }
}
