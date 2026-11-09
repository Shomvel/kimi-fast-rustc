//! Access physical lanes without projecting through a repr(simd) field.

use super::*;

impl<'tcx> Converter<'tcx> {
    pub(super) fn simd_array_view(&mut self, place: Place, vector: SimdType<'tcx>) -> Place {
        let pointer_ty = Ty::new_mut_ptr(self.tcx, vector.ty);
        let pointer = self.intrinsic_temp(pointer_ty, Rvalue::RawPointer(Mutability::Mut, place));
        let array_ty = Ty::new_array(self.tcx, vector.element, vector.lanes);
        let array_pointer_ty = Ty::new_mut_ptr(self.tcx, array_ty);
        let array_pointer_id = self.ty(array_pointer_ty);
        let pointer = self.intrinsic_temp(
            array_pointer_ty,
            Rvalue::Cast(CastKind::PtrToPtr, pointer, array_pointer_id),
        );
        let Operand::Copy(pointer) = pointer else {
            unreachable!()
        };
        Place {
            local: pointer.local,
            projection: self.builder.add_projection(vec![ProjectionElem::Deref]),
        }
    }

    pub(super) fn simd_lane_place(&mut self, array: Place, lanes: u64, index: u64) -> Place {
        let mut projection = self.builder.projection(array.projection).to_vec();
        projection.push(ProjectionElem::ConstantIndex {
            offset: index,
            min_length: lanes,
            from_end: false,
        });
        Place {
            local: array.local,
            projection: self.builder.add_projection(projection),
        }
    }

    pub(super) fn simd_read_lanes(
        &mut self,
        value: Operand,
        vector: SimdType<'tcx>,
    ) -> Vec<Operand> {
        let snapshot = self.intrinsic_temp(vector.ty, Rvalue::Use(value));
        let Operand::Copy(snapshot) = snapshot else {
            unreachable!()
        };
        let array = self.simd_array_view(snapshot, vector);
        (0..vector.lanes)
            .map(|lane| {
                let place = self.simd_lane_place(array, vector.lanes, lane);
                self.intrinsic_temp(vector.element, Rvalue::Use(Operand::Copy(place)))
            })
            .collect()
    }

    pub(super) fn simd_write_lanes(
        &mut self,
        destination: Place,
        vector: SimdType<'tcx>,
        values: &[Operand],
    ) -> Result<(), String> {
        if values.len() as u64 != vector.lanes {
            return Err("SIMD output lane count disagrees with its layout".to_owned());
        }
        let array = self.simd_array_view(destination, vector);
        // Every input lane was snapshotted before this first destination write.
        for (lane, value) in values.iter().copied().enumerate() {
            let place = self.simd_lane_place(array, vector.lanes, lane as u64);
            self.builder
                .push_statement(StatementKind::Assign(place, Rvalue::Use(value)));
        }
        Ok(())
    }

    pub(super) fn simd_nonzero(&mut self, value: Operand, ty: Ty<'tcx>) -> Operand {
        let zero = self.intrinsic_constant(ty, 0);
        self.intrinsic_temp(
            self.tcx.types.bool,
            Rvalue::BinaryOp(BinaryOp::Ne, value, zero),
        )
    }

    pub(super) fn simd_bool_mask(&mut self, value: Operand, ty: Ty<'tcx>) -> Operand {
        let boolean = self.intrinsic_cast(value, ty);
        let zero = self.intrinsic_constant(ty, 0);
        self.intrinsic_temp(ty, Rvalue::BinaryOp(BinaryOp::Sub, zero, boolean))
    }

    pub(super) fn simd_choose(
        &mut self,
        condition: Operand,
        ty: Ty<'tcx>,
        if_true: Operand,
        if_false: Operand,
    ) -> Operand {
        let mask = self.simd_bool_mask(condition, ty);
        let difference =
            self.intrinsic_temp(ty, Rvalue::BinaryOp(BinaryOp::BitXor, if_true, if_false));
        let selected =
            self.intrinsic_temp(ty, Rvalue::BinaryOp(BinaryOp::BitAnd, difference, mask));
        self.intrinsic_temp(ty, Rvalue::BinaryOp(BinaryOp::BitXor, if_false, selected))
    }
}
