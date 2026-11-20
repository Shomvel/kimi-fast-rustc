use super::{Converter, intrinsic_arity};
use kfrc_body::{
    BlockId, Callee, Constant, ConstantKind, FunctionAbi, Operand, PassMode, Place, Terminator,
    UnwindAction,
};
use kfrc_cranelift::AtomicOperation;
use rustc_middle::ty::{self, Instance};

impl<'tcx> Converter<'tcx> {
    pub(super) fn intrinsic_atomic(
        &mut self,
        instance: Instance<'tcx>,
        name: &str,
        args: Vec<Operand>,
        destination: Place,
        target: Option<BlockId>,
        unwind: UnwindAction,
    ) -> Result<Terminator, String> {
        let operation = match name {
            "atomic_load" => AtomicOperation::Load,
            "atomic_store" => AtomicOperation::Store,
            "atomic_xsub" => AtomicOperation::Sub,
            "atomic_xadd" => AtomicOperation::Add,
            "atomic_xchg" => AtomicOperation::Exchange,
            // Strong CAS is a valid implementation of weak CAS (no spurious failure).
            "atomic_cxchg" | "atomic_cxchgweak" => AtomicOperation::CompareExchange,
            "atomic_fence" | "atomic_singlethreadfence" => AtomicOperation::Fence,
            _ => return Err(format!("unsupported atomic intrinsic `{name}`")),
        };
        let arity = match operation {
            AtomicOperation::Load => 1,
            AtomicOperation::Store
            | AtomicOperation::Sub
            | AtomicOperation::Add
            | AtomicOperation::Exchange => 2,
            AtomicOperation::CompareExchange => 3,
            AtomicOperation::Fence => 0,
        };
        intrinsic_arity(name, &args, arity)?;
        let (bits, signed) = if operation == AtomicOperation::Fence {
            (64, false)
        } else {
            let ty = instance.args.type_at(0);
            if !matches!(ty.kind(), ty::Int(_) | ty::Uint(_) | ty::RawPtr(..)) {
                return Err(format!(
                    "atomic intrinsic `{name}` requires integer or thin pointer, got `{ty}`"
                ));
            }
            let bits = self.intrinsic_layout(ty)?.0 * 8;
            if !matches!(bits, 8 | 16 | 32 | 64) {
                return Err(format!(
                    "{bits}-bit atomic intrinsic `{name}` is not implemented"
                ));
            }
            (bits as u8, matches!(ty.kind(), ty::Int(_)))
        };
        let fn_ty = self.ty(self.tcx.types.unit);
        let callee = self.builder.add_constant(Constant {
            ty: fn_ty,
            kind: ConstantKind::Function(Callee::Function {
                symbol: crate::atomic::symbol(self.tcx, operation, bits, signed),
                abi: FunctionAbi {
                    args: vec![PassMode::Direct; args.len()],
                    ret: match operation {
                        AtomicOperation::Store | AtomicOperation::Fence => PassMode::Ignore,
                        AtomicOperation::CompareExchange => PassMode::Pair,
                        _ => PassMode::Direct,
                    },
                    c_variadic: false,
                },
            }),
        });
        Ok(Terminator::Call {
            callee: Operand::Constant(callee),
            args,
            destination,
            target,
            unwind,
        })
    }
}
