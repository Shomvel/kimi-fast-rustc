//! SIMD mask reductions and bitmask/select operations.

use super::*;

impl<'tcx> Converter<'tcx> {
    pub(super) fn simd_bitmask(
        &mut self,
        input: SimdType<'tcx>,
        value: Operand,
        result_ty: Ty<'tcx>,
        destination: Place,
    ) -> Result<(), String> {
        let expected_bits = input.lanes.max(8).next_power_of_two();
        let expected_bytes = input.lanes.div_ceil(8);
        let byte_array = match result_ty.kind() {
            ty::Uint(_) if self.intrinsic_layout(result_ty)?.0 * 8 == expected_bits => false,
            ty::Array(element, length)
                if *element == self.tcx.types.u8
                    && length.try_to_target_usize(self.tcx) == Some(expected_bytes) =>
            {
                true
            }
            _ => return Err(format!("invalid SIMD bitmask result type `{result_ty}`")),
        };
        if !byte_array && expected_bits > 64 {
            return Err("SIMD integer bitmasks wider than 64 bits are not implemented".to_owned());
        }
        let (unsigned, _) = self.intrinsic_unsigned(input.element)?;
        let lanes = self.simd_read_lanes(value, input);
        let output_element = if byte_array {
            self.tcx.types.u8
        } else {
            result_ty
        };
        let count = if byte_array { expected_bytes } else { 1 };
        let mut outputs = (0..count)
            .map(|_| self.intrinsic_constant(output_element, 0))
            .collect::<Vec<_>>();
        for (index, lane) in lanes.into_iter().enumerate() {
            // LLVM uses each lane's high bit. Cast first so unsigned mask
            // lanes and signed mask lanes retain the same physical bits.
            let lane = self.intrinsic_cast(lane, unsigned);
            let shift = self.intrinsic_constant(unsigned, u128::from(input.bits - 1));
            let high = self.intrinsic_temp(unsigned, Rvalue::BinaryOp(BinaryOp::Shr, lane, shift));
            let high = self.intrinsic_cast(high, output_element);
            let output = if byte_array { index / 8 } else { 0 };
            let bit = if byte_array { index % 8 } else { index };
            let shift = self.intrinsic_constant(output_element, bit as u128);
            let high =
                self.intrinsic_temp(output_element, Rvalue::BinaryOp(BinaryOp::Shl, high, shift));
            outputs[output] = self.intrinsic_temp(
                output_element,
                Rvalue::BinaryOp(BinaryOp::BitOr, outputs[output], high),
            );
        }
        let value = if byte_array {
            let element = self.ty(output_element);
            Rvalue::Aggregate(kfrc_body::AggregateKind::Array(element), outputs)
        } else {
            Rvalue::Use(outputs[0])
        };
        self.builder
            .push_statement(StatementKind::Assign(destination, value));
        Ok(())
    }

    pub(super) fn simd_reduce_bool(
        &mut self,
        input: SimdType<'tcx>,
        value: Operand,
        all: bool,
        destination: Place,
    ) -> Result<(), String> {
        let lanes = self.simd_read_lanes(value, input);
        let mut result = self.intrinsic_constant(self.tcx.types.bool, u128::from(all));
        for lane in lanes {
            let nonzero = self.simd_nonzero(lane, input.element);
            result = self.intrinsic_temp(
                self.tcx.types.bool,
                Rvalue::BinaryOp(
                    if all {
                        BinaryOp::BitAnd
                    } else {
                        BinaryOp::BitOr
                    },
                    result,
                    nonzero,
                ),
            );
        }
        self.builder
            .push_statement(StatementKind::Assign(destination, Rvalue::Use(result)));
        Ok(())
    }

    pub(super) fn simd_select(
        &mut self,
        mask: SimdType<'tcx>,
        result: SimdType<'tcx>,
        args: &[Operand],
        destination: Place,
    ) -> Result<(), String> {
        if mask.lanes != result.lanes || !matches!(mask.element.kind(), ty::Int(_)) {
            return Err("SIMD select requires one signed mask lane per result lane".to_owned());
        }
        let masks = self.simd_read_lanes(args[0], mask);
        let masks = masks
            .into_iter()
            .map(|lane| self.simd_nonzero(lane, mask.element))
            .collect::<Vec<_>>();
        self.simd_select_lanes(result, args, masks, destination)
    }

    pub(super) fn simd_select_bitmask(
        &mut self,
        mask_ty: Ty<'tcx>,
        result: SimdType<'tcx>,
        args: &[Operand],
        destination: Place,
    ) -> Result<(), String> {
        let expected_bits = result.lanes.max(8).next_power_of_two();
        let expected_bytes = result.lanes.div_ceil(8);
        let mask_lanes = match mask_ty.kind() {
            ty::Uint(_) if self.intrinsic_layout(mask_ty)?.0 * 8 == expected_bits => {
                if expected_bits > 64 {
                    return Err("SIMD select integer bitmask exceeds 64 bits".to_owned());
                }
                let mask = self.intrinsic_temp(mask_ty, Rvalue::Use(args[0]));
                vec![mask; result.lanes as usize]
            }
            ty::Array(element, length)
                if *element == self.tcx.types.u8
                    && length.try_to_target_usize(self.tcx) == Some(expected_bytes) =>
            {
                let snapshot = self.intrinsic_temp(mask_ty, Rvalue::Use(args[0]));
                let Operand::Copy(array) = snapshot else {
                    unreachable!()
                };
                let mut values = Vec::with_capacity(result.lanes as usize);
                for index in 0..result.lanes {
                    let place = self.simd_lane_place(array, expected_bytes, index / 8);
                    values.push(
                        self.intrinsic_temp(self.tcx.types.u8, Rvalue::Use(Operand::Copy(place))),
                    );
                }
                values
            }
            _ => return Err(format!("invalid SIMD select bitmask type `{mask_ty}`")),
        };
        let byte_array = matches!(mask_ty.kind(), ty::Array(..));
        let lane_ty = if byte_array {
            self.tcx.types.u8
        } else {
            mask_ty
        };
        let mut masks = Vec::with_capacity(mask_lanes.len());
        for (index, value) in mask_lanes.into_iter().enumerate() {
            let bit_index = if byte_array { index % 8 } else { index };
            let shift = self.intrinsic_constant(lane_ty, bit_index as u128);
            let bit = self.intrinsic_temp(lane_ty, Rvalue::BinaryOp(BinaryOp::Shr, value, shift));
            let one = self.intrinsic_constant(lane_ty, 1);
            let bit = self.intrinsic_temp(lane_ty, Rvalue::BinaryOp(BinaryOp::BitAnd, bit, one));
            masks.push(self.simd_nonzero(bit, lane_ty));
        }
        self.simd_select_lanes(result, args, masks, destination)
    }

    fn simd_select_lanes(
        &mut self,
        result: SimdType<'tcx>,
        args: &[Operand],
        masks: Vec<Operand>,
        destination: Place,
    ) -> Result<(), String> {
        let left = self.simd_read_lanes(args[1], result);
        let right = self.simd_read_lanes(args[2], result);
        let values = masks
            .into_iter()
            .zip(left.into_iter().zip(right))
            .map(|(mask, (left, right))| self.simd_choose(mask, result.element, left, right))
            .collect::<Vec<_>>();
        self.simd_write_lanes(destination, result, &values)
    }
}
