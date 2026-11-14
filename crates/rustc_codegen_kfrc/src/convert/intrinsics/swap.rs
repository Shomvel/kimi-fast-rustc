//! Snapshot the representation of both nonoverlapping typed-swap operands.

use super::*;

impl<'tcx> Converter<'tcx> {
    pub(super) fn intrinsic_typed_swap(
        &mut self,
        ty: Ty<'tcx>,
        left: Operand,
        right: Operand,
    ) -> Result<(), String> {
        let size = self.intrinsic_layout(ty)?.0;
        if size == 0 {
            return Ok(());
        }
        // Do not model an aggregate read as an address alias, or interpret
        // inactive enum/union bytes through a field's Rust type. Raw copies
        // also preserve initialized padding without requiring it to be a
        // valid scalar. Each temporary is only addressed, never read as an
        // initialized [u8; N] value before memcpy writes its representation.
        let bytes = Ty::new_array(self.tcx, self.tcx.types.u8, size);
        let bytes_id = self.ty(bytes);
        let pointer_ty = Ty::new_mut_ptr(self.tcx, bytes);
        let mut scratch = || {
            let local = self.builder.add_local(LocalDecl {
                ty: bytes_id,
                mutability: Mutability::Mut,
                name: None,
            });
            self.intrinsic_temp(
                pointer_ty,
                Rvalue::RawPointer(Mutability::Mut, Place::local(local)),
            )
        };
        let left_snapshot = scratch();
        let right_snapshot = scratch();
        // Copy representations through a common byte pointee. The semantic
        // copy node derives its stride from the source type, not a byte-size
        // payload; both endpoints must therefore have this same pointee.
        let byte_pointer = Ty::new_mut_ptr(self.tcx, self.tcx.types.u8);
        let byte_pointer_id = self.ty(byte_pointer);
        let mut byte_address = |operand| {
            self.intrinsic_temp(
                byte_pointer,
                Rvalue::Cast(CastKind::PtrToPtr, operand, byte_pointer_id),
            )
        };
        let left = byte_address(left);
        let right = byte_address(right);
        let left_snapshot = byte_address(left_snapshot);
        let right_snapshot = byte_address(right_snapshot);
        let count = self.intrinsic_constant(self.tcx.types.usize, size as u128);
        for (src, dst) in [
            (left, left_snapshot),
            (right, right_snapshot),
            (right_snapshot, left),
            (left_snapshot, right),
        ] {
            self.builder
                .push_statement(StatementKind::CopyNonOverlapping {
                    src,
                    dst,
                    count,
                    element_size: 1,
                });
        }
        Ok(())
    }
}
