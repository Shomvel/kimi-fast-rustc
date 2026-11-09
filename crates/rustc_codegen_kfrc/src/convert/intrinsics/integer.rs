//! Preserve integer intrinsic semantics until target instruction selection.

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
        // Retain the existing explicit rejection above 64 bits. Do not cast or
        // mask the amount: unchecked funnel must retain its out-of-range UB.
        self.intrinsic_unsigned(ty)?;
        let (hi, lo, amount) = if rotate {
            // MIR may move the input. Consume it once before using its value
            // as both funnel halves; duplicating the Move reads an empty slot.
            let value = self.intrinsic_temp(ty, Rvalue::Use(args[0]));
            (value, value, args[1])
        } else {
            (args[0], args[1], args[2])
        };
        let operation = integer_operation(name)
            .ok_or_else(|| format!("unsupported funnel intrinsic `{name}`"))?;
        Ok(self.intrinsic_temp(
            ty,
            Rvalue::IntegerIntrinsic(operation, vec![hi, lo, amount]),
        ))
    }

    pub(super) fn intrinsic_bits(
        &mut self,
        name: &str,
        ty: Ty<'tcx>,
        args: &[Operand],
    ) -> Result<Operand, String> {
        intrinsic_arity(name, args, 1)?;
        self.intrinsic_unsigned(ty)?;
        let operation =
            integer_operation(name).ok_or_else(|| format!("unsupported bit intrinsic `{name}`"))?;
        let result = if matches!(operation, IntegerIntrinsic::BitCount { .. }) {
            self.tcx.types.u32
        } else {
            ty
        };
        Ok(self.intrinsic_temp(result, Rvalue::IntegerIntrinsic(operation, args.to_vec())))
    }
}
