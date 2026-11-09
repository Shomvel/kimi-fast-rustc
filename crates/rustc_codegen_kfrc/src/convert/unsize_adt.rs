//! Reconstruct CoerceUnsized wrappers field by field, including unchanged data.
//!
//! This follows rustc_codegen_ssa::base::coerce_unsized_into: Box, Unique,
//! NonNull and user wrappers recurse through their actual ADT field layouts.

use super::Converter;
use kfrc_body::{
    AggregateKind, Constant, ConstantKind, FieldIndex, Operand, Place, ProjectionElem, Rvalue,
    VariantIndex,
};
use rustc_middle::ty::{self, Ty, TypingEnv};

impl<'tcx> Converter<'tcx> {
    pub(super) fn unsize_adt(
        &mut self,
        value: Operand,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
    ) -> Result<Rvalue, String> {
        let (ty::Adt(source_def, _), ty::Adt(target_def, _)) = (source.kind(), target.kind())
        else {
            return Err("wrapper coercion requires two ADT types".into());
        };
        if source_def != target_def || !source_def.is_struct() {
            return Err(format!(
                "unsizing unrelated wrappers `{source}` to `{target}`"
            ));
        }
        let source_id = self.ty(source);
        let target_id = self.ty(target);
        let source_place = match value {
            Operand::Copy(place) | Operand::Move(place) => place,
            Operand::Constant(_) => {
                let Operand::Copy(place) = self.cast_temporary(source_id, Rvalue::Use(value))
                else {
                    unreachable!()
                };
                place
            }
        };
        let cx =
            rustc_middle::ty::layout::LayoutCx::new(self.tcx, TypingEnv::fully_monomorphized());
        let source_layout = self
            .tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(source))
            .map_err(|error| format!("source wrapper `{source}` layout: {error:?}"))?;
        let target_layout = self
            .tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(target))
            .map_err(|error| format!("target wrapper `{target}` layout: {error:?}"))?;
        let mut operands = Vec::new();
        for index in 0..target_layout.fields.count() {
            let source_field = source_layout.field(&cx, index);
            let target_field = target_layout.field(&cx, index);
            let target_field_id = self.ty(target_field.ty);
            if target_field.size.bytes() == 0 {
                // Retain the field in the logical aggregate even though it
                // contributes no bytes. This includes allocator/PhantomData.
                operands.push(Operand::Constant(self.builder.add_constant(Constant {
                    ty: target_field_id,
                    kind: ConstantKind::ZeroSized,
                })));
                continue;
            }
            let source_field_id = self.ty(source_field.ty);
            let mut projection = self.builder.projection(source_place.projection).to_vec();
            projection.push(ProjectionElem::Field(
                FieldIndex(index as u32),
                source_field_id,
            ));
            let field_value = Operand::Copy(Place {
                local: source_place.local,
                projection: self.builder.add_projection(projection),
            });
            if source_field.ty == target_field.ty {
                operands.push(field_value);
            } else {
                let coercion =
                    self.unsize_pointer(field_value, source_field.ty, target_field.ty)?;
                operands.push(self.cast_temporary(target_field_id, coercion));
            }
        }
        Ok(Rvalue::Aggregate(
            AggregateKind::Adt {
                ty: target_id,
                variant: VariantIndex(0),
            },
            operands,
        ))
    }
}
