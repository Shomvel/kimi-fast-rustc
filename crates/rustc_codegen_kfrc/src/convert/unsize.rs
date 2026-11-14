//! Build pointer metadata from rustc's array and trait-object layout answers.

use super::{Converter, casts::pointer_pointee};
use kfrc_body::{
    AggregateKind, CastKind, Constant, ConstantKind, FieldIndex, Operand, Place, ProjectionElem,
    Rvalue, TypeOracle, TypeShape,
};
use rustc_middle::ty::{self, Ty, TypingEnv};

impl<'tcx> Converter<'tcx> {
    pub(super) fn unsize_pointer(
        &mut self,
        value: Operand,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
    ) -> Result<Rvalue, String> {
        if matches!((source.kind(), target.kind()), (ty::Adt(..), ty::Adt(..))) {
            return self.unsize_adt(value, source, target);
        }
        let (Some(source_pointee), Some(target_pointee)) =
            (pointer_pointee(source), pointer_pointee(target))
        else {
            return Err(format!(
                "unsizing `{source}` to `{target}` is not a pointer coercion"
            ));
        };
        let (source_pointee, target_pointee) = self.tcx.struct_lockstep_tails_for_codegen(
            source_pointee,
            target_pointee,
            TypingEnv::fully_monomorphized(),
        );
        let target_ty = self.ty(target);
        let target_fields = self
            .builder
            .types()
            .aggregates
            .get(&target_ty)
            .ok_or_else(|| format!("no recorded pointer-pair fields for `{target}`"))?
            .fields
            .clone();
        if target_fields.len() != 2 {
            return Err(format!(
                "pointer-pair layout for `{target}` must contain two fields"
            ));
        }
        let (data, metadata) = match (source_pointee.kind(), target_pointee.kind()) {
            (ty::Array(_, length), ty::Slice(_)) => {
                let length = length
                    .try_to_target_usize(self.tcx)
                    .ok_or_else(|| "unsized array length is not monomorphic".to_owned())?;
                let metadata = Operand::Constant(self.builder.add_constant(Constant {
                    ty: target_fields[1].ty,
                    kind: ConstantKind::Scalar(u128::from(length)),
                }));
                (value, metadata)
            }
            (ty::Dynamic(source_traits, _), ty::Dynamic(target_traits, _)) => {
                // Removing auto traits retains the same vtable. A real
                // supertrait upcast may need a TraitVPtr load; reject it until
                // that path is implemented rather than copying wrong metadata.
                if source_traits.principal_def_id() != target_traits.principal_def_id()
                    && target_traits.principal_def_id().is_some()
                {
                    return Err(format!(
                        "trait-object supertrait upcast from `{source}` to `{target}` is not supported yet"
                    ));
                }
                let source_ty = self.ty(source);
                let fields = self
                    .builder
                    .types()
                    .aggregates
                    .get(&source_ty)
                    .ok_or_else(|| format!("no recorded pointer-pair fields for `{source}`"))?
                    .fields
                    .clone();
                if fields.len() != 2 {
                    return Err(format!(
                        "pointer-pair layout for `{source}` must contain two fields"
                    ));
                }
                let place = match value {
                    Operand::Copy(place) | Operand::Move(place) => place,
                    Operand::Constant(_) => {
                        let Operand::Copy(place) =
                            self.cast_temporary(source_ty, Rvalue::Use(value))
                        else {
                            unreachable!()
                        };
                        place
                    }
                };
                let mut project = |field: usize| {
                    let mut projection = self.builder.projection(place.projection).to_vec();
                    projection.push(ProjectionElem::Field(
                        FieldIndex(field as u32),
                        fields[field].ty,
                    ));
                    Operand::Copy(Place {
                        local: place.local,
                        projection: self.builder.add_projection(projection),
                    })
                };
                (project(0), project(1))
            }
            (_, ty::Dynamic(traits, _)) => {
                if !source_pointee.is_sized(self.tcx, TypingEnv::fully_monomorphized()) {
                    return Err(format!(
                        "unsized concrete source `{source_pointee}` cannot form this trait object"
                    ));
                }
                let principal = traits
                    .principal()
                    .map(|principal| self.tcx.instantiate_bound_regions_with_erased(principal));
                let allocation = self.tcx.vtable_allocation((source_pointee, principal));
                let metadata = self.constants.address(allocation, 0, &mut self.builder)?;
                let metadata = Operand::Constant(self.builder.add_constant(Constant {
                    ty: target_fields[1].ty,
                    kind: metadata,
                }));
                (value, metadata)
            }
            _ => {
                return Err(format!(
                    "unsizing `{source}` to `{target}` is not supported yet"
                ));
            }
        };
        let data = self.cast_temporary(
            target_fields[0].ty,
            Rvalue::Cast(CastKind::PtrToPtr, data, target_fields[0].ty),
        );
        let metadata_cast = match self.builder.types().shape(target_fields[1].ty) {
            TypeShape::RawPointer { .. } | TypeShape::Reference { .. } => CastKind::PtrToPtr,
            _ => CastKind::IntToInt,
        };
        let metadata = self.cast_temporary(
            target_fields[1].ty,
            Rvalue::Cast(metadata_cast, metadata, target_fields[1].ty),
        );
        Ok(Rvalue::Aggregate(
            AggregateKind::Tuple,
            vec![data, metadata],
        ))
    }
}
