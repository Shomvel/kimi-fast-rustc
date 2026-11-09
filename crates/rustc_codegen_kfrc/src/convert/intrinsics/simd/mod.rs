//! Integer SIMD semantics expressed through ordinary typed scalar operations.
//!
//! SIMD values retain rustc's layout and call ABI. Scalarization only affects
//! the intrinsic body; it never changes how a vector crosses a call boundary.

use super::*;

mod lanes;
mod masks;
mod numeric;
mod shuffle;

#[derive(Clone, Copy)]
struct SimdType<'tcx> {
    ty: Ty<'tcx>,
    element: Ty<'tcx>,
    lanes: u64,
    bits: u64,
}

impl<'tcx> Converter<'tcx> {
    pub(super) fn intrinsic_simd(
        &mut self,
        instance: Instance<'tcx>,
        name: &str,
        args: &[Operand],
        mir_args: &[rustc_span::source_map::Spanned<mir::Operand<'tcx>>],
        destination: Place,
    ) -> Result<(), String> {
        let signature = self
            .tcx
            .fn_sig(instance.def_id())
            .instantiate(self.tcx, instance.args)
            .skip_binder();
        let result_ty = self.monomorphize(signature.output());
        if name == "simd_select_bitmask" {
            intrinsic_arity(name, args, 3)?;
            let result = self.simd_type(result_ty)?;
            return self.simd_select_bitmask(instance.args.type_at(0), result, args, destination);
        }
        let input = self.simd_type(instance.args.type_at(0))?;
        match name {
            "simd_and" | "simd_or" | "simd_xor" | "simd_add" | "simd_sub" | "simd_mul"
            | "simd_shl" | "simd_shr" | "simd_eq" | "simd_ne" | "simd_lt" | "simd_le"
            | "simd_gt" | "simd_ge" => {
                intrinsic_arity(name, args, 2)?;
                let result = self.simd_type(result_ty)?;
                self.simd_binary(name, input, result, args, destination)
            }
            "simd_cast" | "simd_as" => {
                intrinsic_arity(name, args, 1)?;
                let result = self.simd_type(result_ty)?;
                self.simd_cast(input, result, args[0], destination)
            }
            "simd_bitmask" => {
                intrinsic_arity(name, args, 1)?;
                self.simd_bitmask(input, args[0], result_ty, destination)
            }
            "simd_reduce_all" | "simd_reduce_any" => {
                intrinsic_arity(name, args, 1)?;
                self.simd_reduce_bool(input, args[0], name == "simd_reduce_all", destination)
            }
            "simd_select" => {
                intrinsic_arity(name, args, 3)?;
                let result = self.simd_type(result_ty)?;
                self.simd_select(input, result, args, destination)
            }
            "simd_shuffle" | "simd_shuffle_const_generic" => {
                intrinsic_arity(name, args, if name == "simd_shuffle" { 3 } else { 2 })?;
                let result = self.simd_type(result_ty)?;
                self.simd_shuffle(instance, input, result, args, mir_args, destination)
            }
            "simd_extract" | "simd_extract_dyn" | "simd_insert" | "simd_insert_dyn" => {
                let insert = name.starts_with("simd_insert");
                intrinsic_arity(name, args, if insert { 3 } else { 2 })?;
                self.simd_lane_access(input, args, insert, destination)
            }
            _ => Err(format!(
                "integer SIMD intrinsic `{name}` is not implemented"
            )),
        }
    }

    fn simd_type(&self, ty: Ty<'tcx>) -> Result<SimdType<'tcx>, String> {
        if !ty.is_simd() {
            return Err(format!("SIMD intrinsic requires a SIMD type, got `{ty}`"));
        }
        let (lanes, element) = ty.simd_size_and_type(self.tcx);
        if lanes == 0 || !matches!(element.kind(), ty::Int(_) | ty::Uint(_)) {
            return Err(format!(
                "SIMD intrinsic requires nonempty integer lanes, got `{ty}`"
            ));
        }
        let bits = self.intrinsic_layout(element)?.0 * 8;
        if !matches!(bits, 8 | 16 | 32 | 64) {
            return Err(format!("SIMD intrinsic has unsupported {bits}-bit lanes"));
        }
        Ok(SimdType {
            ty,
            element,
            lanes,
            bits,
        })
    }
}
