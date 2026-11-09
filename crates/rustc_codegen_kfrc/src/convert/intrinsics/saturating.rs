//! Saturating arithmetic remains one operation at the original Rust width.

use super::*;

impl<'tcx> Converter<'tcx> {
    pub(super) fn intrinsic_saturating(
        &mut self,
        name: &str,
        ty: Ty<'tcx>,
        args: &[Operand],
    ) -> Result<Operand, String> {
        intrinsic_arity(name, args, 2)?;
        self.intrinsic_unsigned(ty)?;
        let operation = integer_operation(name)
            .ok_or_else(|| format!("unsupported saturation intrinsic `{name}`"))?;
        Ok(self.intrinsic_temp(ty, Rvalue::IntegerIntrinsic(operation, args.to_vec())))
    }
}
