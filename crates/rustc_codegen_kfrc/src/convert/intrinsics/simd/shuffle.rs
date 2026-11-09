//! Constant-index SIMD shuffle and dynamic lane access.

use super::*;
use rustc_abi::Size;
use rustc_middle::mir::interpret::alloc_range;

impl<'tcx> Converter<'tcx> {
    pub(super) fn simd_shuffle(
        &mut self,
        instance: Instance<'tcx>,
        input: SimdType<'tcx>,
        result: SimdType<'tcx>,
        args: &[Operand],
        mir_args: &[rustc_span::source_map::Spanned<mir::Operand<'tcx>>],
        destination: Place,
    ) -> Result<(), String> {
        if input.element != result.element {
            return Err("SIMD shuffle changes its lane type".to_owned());
        }
        let indices = if args.len() == 3 {
            let operand = &mir_args
                .get(2)
                .ok_or("SIMD shuffle has no index operand")?
                .node;
            self.shuffle_indices(operand)?
        } else {
            let value = self.monomorphize(instance.args.const_at(2));
            value
                .try_to_branch()
                .ok_or("SIMD shuffle const generic is not an array")?
                .iter()
                .map(|index| {
                    index
                        .try_to_leaf()
                        .map(|index| index.to_u32())
                        .ok_or_else(|| {
                            "SIMD shuffle const generic has a nonscalar index".to_owned()
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        if indices.len() as u64 != result.lanes {
            return Err("SIMD shuffle index count disagrees with result lanes".to_owned());
        }
        let mut joined = self.simd_read_lanes(args[0], input);
        joined.extend(self.simd_read_lanes(args[1], input));
        let mut values = Vec::with_capacity(indices.len());
        for index in indices {
            values.push(*joined.get(index as usize).ok_or_else(|| {
                format!(
                    "SIMD shuffle index {index} exceeds {} lanes",
                    input.lanes * 2
                )
            })?);
        }
        self.simd_write_lanes(destination, result, &values)
    }

    fn shuffle_indices(&self, operand: &mir::Operand<'tcx>) -> Result<Vec<u32>, String> {
        let mir::Operand::Constant(constant) = operand else {
            return Err("SIMD shuffle index operand is not a MIR constant".to_owned());
        };
        let constant_value = self.monomorphize(constant.const_);
        let index_type = self.simd_type(constant_value.ty())?;
        if index_type.element != self.tcx.types.u32 {
            return Err("SIMD shuffle indices must have u32 lanes".to_owned());
        }
        let value = constant_value
            .eval(self.tcx, TypingEnv::fully_monomorphized(), constant.span)
            .map_err(|_| "SIMD shuffle index constant failed CTFE".to_owned())?;
        let mir::ConstValue::Indirect { alloc_id, offset } = value else {
            return Err("SIMD shuffle index vector has no CTFE allocation".to_owned());
        };
        let allocation = self.tcx.global_alloc(alloc_id).unwrap_memory().inner();
        let size = Size::from_bytes(4);
        let end = offset
            .bytes()
            .checked_add(
                index_type
                    .lanes
                    .checked_mul(size.bytes())
                    .ok_or("SIMD shuffle index byte length overflow")?,
            )
            .ok_or("SIMD shuffle index allocation offset overflow")?;
        if end > allocation.size().bytes() {
            return Err("SIMD shuffle indices extend beyond their CTFE allocation".to_owned());
        }
        let mut indices = Vec::with_capacity(index_type.lanes as usize);
        for index in 0..index_type.lanes {
            // CTFE validates initialization/provenance and reads target endian.
            // Padding after the final physical SIMD lane is ignored.
            let scalar = allocation
                .read_scalar(&self.tcx, alloc_range(offset + size * index, size), false)
                .map_err(|error| format!("invalid SIMD shuffle index allocation: {error:?}"))?;
            indices.push(
                scalar
                    .try_to_scalar_int()
                    .map_err(|_| "SIMD shuffle index has pointer provenance".to_owned())?
                    .to_u32(),
            );
        }
        Ok(indices)
    }

    pub(super) fn simd_lane_access(
        &mut self,
        vector: SimdType<'tcx>,
        args: &[Operand],
        insert: bool,
        destination: Place,
    ) -> Result<(), String> {
        let snapshot = self.intrinsic_temp(vector.ty, Rvalue::Use(args[0]));
        let Operand::Copy(snapshot) = snapshot else {
            unreachable!()
        };
        let array = self.simd_array_view(snapshot, vector);
        let index = self.intrinsic_cast(args[1], self.tcx.types.usize);
        let Operand::Copy(index) = index else {
            unreachable!()
        };
        let mut projection = self.builder.projection(array.projection).to_vec();
        projection.push(ProjectionElem::Index(index.local));
        let lane = Place {
            local: array.local,
            projection: self.builder.add_projection(projection),
        };
        if insert {
            let value = self.intrinsic_temp(vector.element, Rvalue::Use(args[2]));
            self.builder
                .push_statement(StatementKind::Assign(lane, Rvalue::Use(value)));
            self.builder.push_statement(StatementKind::Assign(
                destination,
                Rvalue::Use(Operand::Copy(snapshot)),
            ));
        } else {
            self.builder.push_statement(StatementKind::Assign(
                destination,
                Rvalue::Use(Operand::Copy(lane)),
            ));
        }
        Ok(())
    }
}
