//! Place projections, operands, and materialization of evaluated constants.

use super::Converter;
use crate::constants::LoweredConstant;
use kfrc_body::{
    AggregateKind, Constant, ConstantKind, FieldIndex, LocalDecl, Mutability, Operand, Place,
    ProjectionElem, Rvalue, StatementKind, VariantIndex,
};
use rustc_middle::mir;
use rustc_middle::ty::{self, Ty, TypingEnv};

impl<'tcx> Converter<'tcx> {
    pub(super) fn place(&mut self, place: mir::Place<'tcx>) -> Result<Place, String> {
        let mut elements = Vec::new();
        let body = self.tcx.instance_mir(self.instance.def);
        let mut place_ty =
            mir::PlaceTy::from_ty(self.monomorphize(body.local_decls[place.local].ty));
        for element in place.projection {
            let element = self.monomorphize(element);
            place_ty = place_ty.projection_ty(self.tcx, element);
            elements.push(match element {
                mir::ProjectionElem::Field(field, ty) => {
                    let ty = self.ty(self.monomorphize(ty));
                    ProjectionElem::Field(FieldIndex(field.as_u32()), ty)
                }
                mir::ProjectionElem::Deref => ProjectionElem::Deref,
                mir::ProjectionElem::Index(local) => ProjectionElem::Index(self.local_id(local)),
                mir::ProjectionElem::ConstantIndex {
                    offset,
                    min_length,
                    from_end,
                } => ProjectionElem::ConstantIndex {
                    offset,
                    min_length,
                    from_end,
                },
                mir::ProjectionElem::Subslice { from, to, from_end } => ProjectionElem::Subslice {
                    from,
                    to,
                    from_end,
                    result_ty: self.ty(place_ty.ty),
                },
                mir::ProjectionElem::Downcast(_, variant) => {
                    ProjectionElem::Downcast(VariantIndex(variant.as_u32()))
                }
                other => return Err(format!("place projection {other:?}")),
            });
        }
        Ok(Place {
            local: self.local_id(place.local),
            projection: self.builder.add_projection(elements),
        })
    }

    pub(super) fn operand(&mut self, operand: &mir::Operand<'tcx>) -> Result<Operand, String> {
        match operand {
            mir::Operand::Copy(place) => Ok(Operand::Copy(self.place(*place)?)),
            mir::Operand::Move(place) => Ok(Operand::Move(self.place(*place)?)),
            mir::Operand::Constant(constant) => {
                let value = self.monomorphize(constant.const_);
                let ty = value.ty();
                let kind = if let ty::FnDef(..) = *ty.kind() {
                    // A function item carries no address, including virtual
                    // methods passed through generic Fn traits. Direct calls
                    // resolve its nominal type; reification creates an address.
                    ConstantKind::ZeroSized
                } else {
                    let evaluated = value
                        .eval(self.tcx, TypingEnv::fully_monomorphized(), constant.span)
                        .map_err(|_| format!("constant of type `{ty}` failed to evaluate"))?;
                    return self.constant_value(evaluated, ty);
                };
                let ty = self.ty(ty);
                Ok(Operand::Constant(
                    self.builder.add_constant(Constant { ty, kind }),
                ))
            }
            mir::Operand::RuntimeChecks(checks) => {
                let ty = self.ty(self.tcx.types.bool);
                Ok(Operand::Constant(self.builder.add_constant(Constant {
                    ty,
                    kind: ConstantKind::Scalar(u128::from(checks.value(self.tcx.sess))),
                })))
            }
        }
    }

    pub(super) fn constant_value(
        &mut self,
        value: mir::ConstValue,
        ty: Ty<'tcx>,
    ) -> Result<Operand, String> {
        let lowered = self.constants.lower(value, &mut self.builder)?;
        let type_id = self.ty(ty);
        match lowered {
            LoweredConstant::Immediate(kind) => Ok(Operand::Constant(
                self.builder.add_constant(Constant { ty: type_id, kind }),
            )),
            LoweredConstant::Pair { first, second } => {
                let fields = self
                    .builder
                    .types()
                    .aggregates
                    .get(&type_id)
                    .ok_or_else(|| format!("no recorded pair layout for `{ty}`"))?
                    .fields
                    .clone();
                if fields.len() != 2 {
                    return Err(format!(
                        "pair layout for `{ty}` has {} fields",
                        fields.len()
                    ));
                }
                let first = Operand::Constant(self.builder.add_constant(Constant {
                    ty: fields[0].ty,
                    kind: first,
                }));
                let second = Operand::Constant(self.builder.add_constant(Constant {
                    ty: fields[1].ty,
                    kind: second,
                }));
                let local = self.builder.add_local(LocalDecl {
                    ty: type_id,
                    mutability: Mutability::Not,
                    name: None,
                });
                self.builder.push_statement(StatementKind::Assign(
                    Place::local(local),
                    Rvalue::Aggregate(AggregateKind::Tuple, vec![first, second]),
                ));
                Ok(Operand::Copy(Place::local(local)))
            }
            LoweredConstant::ByReference { symbol, offset } => {
                let ptr_ty = self.ty(Ty::new_imm_ptr(self.tcx, ty));
                let pointer = self.builder.add_local(LocalDecl {
                    ty: ptr_ty,
                    mutability: Mutability::Not,
                    name: None,
                });
                let addend =
                    i64::try_from(offset).map_err(|_| "constant allocation offset exceeds i64")?;
                let address = self.builder.add_constant(Constant {
                    ty: ptr_ty,
                    kind: ConstantKind::Address { symbol, addend },
                });
                self.builder.push_statement(StatementKind::Assign(
                    Place::local(pointer),
                    Rvalue::Use(Operand::Constant(address)),
                ));
                let projection = self.builder.add_projection(vec![ProjectionElem::Deref]);
                Ok(Operand::Copy(Place {
                    local: pointer,
                    projection,
                }))
            }
        }
    }

    pub(super) fn caller_location(
        &mut self,
        source_info: mir::SourceInfo,
    ) -> Result<Operand, String> {
        let mir = self.tcx.instance_mir(self.instance.def);
        let tracked = self
            .caller_location
            .map(|local| Operand::Copy(Place::local(local)));
        let span_or_value =
            mir.caller_location_span(source_info, tracked.map(Ok), self.tcx, |span| Err(span));
        match span_or_value {
            Ok(operand) => Ok(operand),
            Err(span) => self.constant_value(
                self.tcx.span_as_caller_location(span),
                self.tcx.caller_location_ty(),
            ),
        }
    }
}
