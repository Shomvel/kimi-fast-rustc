//! Assignments, storage lifetimes, and effectful MIR statement intrinsics.

use super::Converter;
use kfrc_body::{StatementKind, VariantIndex};
use rustc_middle::mir;
use rustc_middle::ty::{self, TypingEnv};

impl<'tcx> Converter<'tcx> {
    pub(super) fn statement(
        &mut self,
        statement: &mir::Statement<'tcx>,
    ) -> Result<Option<StatementKind>, String> {
        Ok(Some(match &statement.kind {
            mir::StatementKind::Assign(assignment) => {
                let (place, rvalue) = &**assignment;
                StatementKind::Assign(self.place(*place)?, self.rvalue(rvalue)?)
            }
            mir::StatementKind::StorageLive(local) => {
                StatementKind::StorageLive(self.local_id(*local))
            }
            mir::StatementKind::StorageDead(local) => {
                StatementKind::StorageDead(self.local_id(*local))
            }
            mir::StatementKind::SetDiscriminant {
                place,
                variant_index,
            } => StatementKind::SetDiscriminant(
                self.place(**place)?,
                VariantIndex(variant_index.as_u32()),
            ),
            mir::StatementKind::Intrinsic(intrinsic) => match &**intrinsic {
                mir::NonDivergingIntrinsic::Assume(predicate) => {
                    StatementKind::Assume(self.operand(predicate)?)
                }
                mir::NonDivergingIntrinsic::CopyNonOverlapping(copy) => {
                    let body = self.tcx.instance_mir(self.instance.def);
                    let pointer_ty = self.monomorphize(copy.src.ty(&body.local_decls, self.tcx));
                    let element_ty = match *pointer_ty.kind() {
                        ty::RawPtr(pointee, _) | ty::Ref(_, pointee, _) => pointee,
                        _ => {
                            return Err(format!(
                                "copy_nonoverlapping source has non-pointer type `{pointer_ty}`"
                            ));
                        }
                    };
                    let element_size = self
                        .tcx
                        .layout_of(TypingEnv::fully_monomorphized().as_query_input(element_ty))
                        .map_err(|error| format!("copy_nonoverlapping element layout: {error:?}"))?
                        .size
                        .bytes();
                    StatementKind::CopyNonOverlapping {
                        src: self.operand(&copy.src)?,
                        dst: self.operand(&copy.dst)?,
                        count: self.operand(&copy.count)?,
                        element_size,
                    }
                }
            },
            // Checker-only and debugging statements carry no runtime semantics.
            mir::StatementKind::FakeRead(_)
            | mir::StatementKind::Retag(..)
            | mir::StatementKind::PlaceMention(_)
            | mir::StatementKind::AscribeUserType(..)
            | mir::StatementKind::Coverage(_)
            | mir::StatementKind::ConstEvalCounter
            | mir::StatementKind::BackwardIncompatibleDropHint { .. }
            | mir::StatementKind::Nop => return Ok(None),
        }))
    }
}
