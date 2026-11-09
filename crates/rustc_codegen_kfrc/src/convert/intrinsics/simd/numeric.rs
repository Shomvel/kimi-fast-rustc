//! Lane-wise integer arithmetic, bit operations and comparison masks.

use super::*;

impl<'tcx> Converter<'tcx> {
    pub(super) fn simd_binary(
        &mut self,
        name: &str,
        input: SimdType<'tcx>,
        result: SimdType<'tcx>,
        args: &[Operand],
        destination: Place,
    ) -> Result<(), String> {
        if input.lanes != result.lanes {
            return Err("SIMD binary operation has different input/output lengths".to_owned());
        }
        let (op, comparison) = match name {
            "simd_and" => (BinaryOp::BitAnd, false),
            "simd_or" => (BinaryOp::BitOr, false),
            "simd_xor" => (BinaryOp::BitXor, false),
            "simd_add" => (BinaryOp::Add, false),
            "simd_sub" => (BinaryOp::Sub, false),
            "simd_mul" => (BinaryOp::Mul, false),
            "simd_shl" => (BinaryOp::Shl, false),
            "simd_shr" => (BinaryOp::Shr, false),
            "simd_eq" => (BinaryOp::Eq, true),
            "simd_ne" => (BinaryOp::Ne, true),
            "simd_lt" => (BinaryOp::Lt, true),
            "simd_le" => (BinaryOp::Le, true),
            "simd_gt" => (BinaryOp::Gt, true),
            "simd_ge" => (BinaryOp::Ge, true),
            _ => unreachable!(),
        };
        if (!comparison && input.element != result.element)
            || (comparison
                && (input.bits != result.bits || !matches!(result.element.kind(), ty::Int(_))))
        {
            return Err("SIMD operation has incompatible result lane type".to_owned());
        }
        let left = self.simd_read_lanes(args[0], input);
        let right = self.simd_read_lanes(args[1], input);
        let mut values = Vec::with_capacity(left.len());
        for (left, right) in left.into_iter().zip(right) {
            let ty = if comparison {
                self.tcx.types.bool
            } else {
                input.element
            };
            let value = self.intrinsic_temp(ty, Rvalue::BinaryOp(op, left, right));
            values.push(if comparison {
                self.simd_bool_mask(value, result.element)
            } else {
                value
            });
        }
        self.simd_write_lanes(destination, result, &values)
    }

    pub(super) fn simd_cast(
        &mut self,
        input: SimdType<'tcx>,
        result: SimdType<'tcx>,
        value: Operand,
        destination: Place,
    ) -> Result<(), String> {
        if input.lanes != result.lanes {
            return Err("SIMD cast has different input/output lengths".to_owned());
        }
        let lanes = self.simd_read_lanes(value, input);
        let lanes = lanes
            .into_iter()
            .map(|lane| self.intrinsic_cast(lane, result.element))
            .collect::<Vec<_>>();
        self.simd_write_lanes(destination, result, &lanes)
    }
}
