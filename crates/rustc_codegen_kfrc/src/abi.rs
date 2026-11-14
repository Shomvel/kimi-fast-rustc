//! Rust ABI queries and the MIR-to-ABI argument correspondence.
//!
//! MIR retains the tuple argument of `extern "rust-call"` functions, while
//! rustc's `FnAbi` spreads that tuple into separate arguments. This mapping is
//! explicit so the converter can reconstruct the MIR local on entry and use
//! exactly the same ordering at call sites. An implicit caller location comes
//! after all spread arguments.

use kfrc_body::{FunctionAbi, PassMode};
use rustc_abi::ExternAbi;
use rustc_middle::mir;
use rustc_middle::ty::{self, Instance, PolyFnSig, Ty, TyCtxt, TypingEnv};
use rustc_target::callconv;

mod cast;
mod vector;

pub struct ArgumentPart<'tcx> {
    /// `None` is rustc's implicit `#[track_caller]` argument.
    pub local: Option<mir::Local>,
    /// A field of `local` if MIR retains a tuple that the ABI spreads.
    pub field: Option<usize>,
    pub ty: Ty<'tcx>,
}

pub struct DefinitionArgs<'tcx> {
    /// One entry per `FunctionAbi::args`, including ignored zero-sized values.
    pub parts: Vec<ArgumentPart<'tcx>>,
    pub spread_local: Option<mir::Local>,
    /// Zero-based index into `parts`, before lowering each `PassMode`.
    pub caller_location_index: Option<usize>,
}

pub fn instance_abi<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
) -> Result<FunctionAbi, String> {
    let abi = tcx
        .fn_abi_of_instance(
            TypingEnv::fully_monomorphized().as_query_input((instance, ty::List::empty())),
        )
        .map_err(|error| format!("no ABI for `{instance}`: {error:?}"))?;
    record_abi(tcx, abi)
}

/// Function-pointer calls use rustc's dedicated query. Querying an invented
/// instance would lose the function pointer's calling convention and signature.
pub fn fn_pointer_abi<'tcx>(
    tcx: TyCtxt<'tcx>,
    signature: PolyFnSig<'tcx>,
) -> Result<FunctionAbi, String> {
    let abi = tcx
        .fn_abi_of_fn_ptr(
            TypingEnv::fully_monomorphized().as_query_input((signature, ty::List::empty())),
        )
        .map_err(|error| format!("no ABI for function pointer `{signature}`: {error:?}"))?;
    record_abi(tcx, abi)
}

pub fn definition_args<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    body: &mir::Body<'tcx>,
) -> Result<DefinitionArgs<'tcx>, String> {
    let mut parts = Vec::new();
    for local in body.args_iter() {
        let ty = instance.instantiate_mir_and_normalize_erasing_regions(
            tcx,
            TypingEnv::fully_monomorphized(),
            ty::EarlyBinder::bind(body.local_decls[local].ty),
        );
        if Some(local) == body.spread_arg {
            let ty::Tuple(fields) = ty.kind() else {
                return Err(format!(
                    "spread argument {local:?} of `{instance}` is not a tuple: `{ty}`"
                ));
            };
            parts.extend(fields.iter().enumerate().map(|(field, ty)| ArgumentPart {
                local: Some(local),
                field: Some(field),
                ty,
            }));
        } else {
            parts.push(ArgumentPart {
                local: Some(local),
                field: None,
                ty,
            });
        }
    }
    let caller_location_index = if instance.def.requires_caller_location(tcx) {
        let index = parts.len();
        parts.push(ArgumentPart {
            local: None,
            field: None,
            ty: tcx.caller_location_ty(),
        });
        Some(index)
    } else {
        None
    };
    let abi = instance_abi(tcx, instance)?;
    if parts.len() != abi.args.len() {
        return Err(format!(
            "argument normalization for `{instance}` produced {} arguments but rustc's ABI has {}",
            parts.len(),
            abi.args.len()
        ));
    }
    Ok(DefinitionArgs {
        parts,
        spread_local: body.spread_arg,
        caller_location_index,
    })
}

/// The *last* source argument must be unpacked exactly once for RustCall.
/// This intentionally does not infer spreading from a tuple type alone: normal
/// Rust functions may also take tuples without spreading their ABI argument.
pub fn call_tuple_fields<'tcx>(
    tcx: TyCtxt<'tcx>,
    function_ty: Ty<'tcx>,
) -> Result<Option<Vec<Ty<'tcx>>>, String> {
    if !function_ty.is_fn() {
        return Err(format!("call target has non-function type `{function_ty}`"));
    }
    let signature = function_ty.fn_sig(tcx);
    if signature.abi() != ExternAbi::RustCall {
        return Ok(None);
    }
    let signature = signature.skip_binder();
    let Some(tuple) = signature.inputs().last() else {
        return Err("rust-call signature has no tuple argument".to_owned());
    };
    let ty::Tuple(fields) = tuple.kind() else {
        return Err(format!("last rust-call argument is not a tuple: `{tuple}`"));
    };
    Ok(Some(fields.iter().collect()))
}

fn record_abi<'tcx>(
    tcx: TyCtxt<'tcx>,
    abi: &callconv::FnAbi<'tcx, Ty<'tcx>>,
) -> Result<FunctionAbi, String> {
    if abi.c_variadic {
        return Err("C-variadic functions are not implemented".to_owned());
    }
    Ok(FunctionAbi {
        args: abi
            .args
            .iter()
            .map(|arg| pass_mode(tcx, arg))
            .collect::<Result<_, _>>()?,
        ret: pass_mode(tcx, &abi.ret)?,
        c_variadic: false,
    })
}

fn pass_mode<'tcx>(
    tcx: TyCtxt<'tcx>,
    argument: &callconv::ArgAbi<'tcx, Ty<'tcx>>,
) -> Result<PassMode, String> {
    Ok(match &argument.mode {
        callconv::PassMode::Ignore => PassMode::Ignore,
        callconv::PassMode::Direct(_)
            if matches!(
                argument.layout.backend_repr,
                rustc_abi::BackendRepr::SimdVector { .. }
            ) =>
        {
            vector::direct(argument.layout.size.bytes())?
        }
        callconv::PassMode::Direct(_) => PassMode::Direct,
        callconv::PassMode::Pair(..) => PassMode::Pair,
        callconv::PassMode::Cast { pad_i32, cast } => {
            if let Some(mode) = vector::cast(argument.layout.size.bytes(), *pad_i32, cast)? {
                mode
            } else {
                PassMode::Cast(cast::record_cast(
                    tcx,
                    argument.layout.size.bytes(),
                    *pad_i32,
                    cast,
                )?)
            }
        }
        callconv::PassMode::Indirect {
            meta_attrs: Some(_),
            ..
        } => {
            return Err(format!(
                "unsized indirect ABI argument `{}`",
                argument.layout.ty
            ));
        }
        callconv::PassMode::Indirect {
            on_stack: false, ..
        } => PassMode::Indirect,
        callconv::PassMode::Indirect { on_stack: true, .. } => PassMode::IndirectOnStack,
    })
}
