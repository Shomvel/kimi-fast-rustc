//! RS5.1 borrow permissions of direct parameters (rustc-O contract, D1).
//!
//! rustc 1.94.1 computes `frozen`/`unpin` only when optimizing
//! (`ty_and_layout_pointee_info_at`), so most PoC builds (`-Copt-level=0`)
//! carry no `noalias`/`readonly` attributes. kfrc evaluates the same type
//! predicates for the same instance instead: `&T` with `T: Freeze` is
//! `Frozen`; `&mut T` with `T: Unpin` under `-Zmutable-noalias` is `Unique`.
//! Every other argument, including `Box`, raw pointers and references inside
//! aggregates, is `None`. When the session optimizes, each permission is
//! checked against rustc's own `ArgAttributes` (the N5.14 guard).

use std::collections::BTreeMap;

use kfrc_son::ArgPermission;
use rustc_middle::mir;
use rustc_middle::ty::{self, Instance, InstanceKind, Ty, TyCtxt, TypingEnv};
use rustc_session::config::OptLevel;
use rustc_target::callconv::{self, ArgAttribute};

/// `KFRC_ARG_PERMISSIONS=off|rustc-o` (default `rustc-o`) selects the A/B arm.
pub fn enabled() -> Result<bool, String> {
    match std::env::var("KFRC_ARG_PERMISSIONS").as_deref() {
        Err(_) | Ok("rustc-o") => Ok(true),
        Ok("off") => Ok(false),
        Ok(other) => Err(format!(
            "KFRC_ARG_PERMISSIONS must be `off` or `rustc-o`, not `{other}`"
        )),
    }
}

#[derive(Default)]
pub struct PermissionTotals {
    pub parameters: usize,
    pub frozen: usize,
    pub unique: usize,
    /// Parameters rustc marks `noalias`; counted only when optimizing.
    pub rustc_noalias: usize,
    pub guarded: usize,
}

impl PermissionTotals {
    pub fn report(&self, crate_name: &str) {
        if std::env::var_os("KFRC_PERMISSION_STATS").is_some() {
            eprintln!(
                "kfrc permission totals {crate_name}: parameters={} frozen={} unique={} rustc_noalias={} guarded={}",
                self.parameters, self.frozen, self.unique, self.rustc_noalias, self.guarded
            );
        }
    }
}

/// Permissions of an instance's S parameters. The semantic builder creates one
/// `Start` parameter per rustc ABI argument (spread tuple fields and the caller
/// location included), so the ABI argument order is the parameter order.
pub fn definition_permissions<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    totals: &mut PermissionTotals,
) -> Result<Vec<ArgPermission>, String> {
    let abi = fn_abi(tcx, instance)?;
    let permissions = abi_permissions(tcx, abi);
    totals.parameters += permissions.len();
    totals.frozen += permissions
        .iter()
        .filter(|permission| permission.is_frozen())
        .count();
    totals.unique += permissions
        .iter()
        .filter(|permission| matches!(permission, ArgPermission::Unique { .. }))
        .count();
    if tcx.sess.opts.optimize != OptLevel::No {
        totals.rustc_noalias += abi
            .args
            .iter()
            .filter(|argument| {
                argument_attributes(&argument.mode)
                    .is_some_and(|attributes| attributes.regular.contains(ArgAttribute::NoAlias))
            })
            .count();
        totals.guarded += guard(abi, &permissions)
            .map_err(|error| format!("permission guard for `{instance}`: {error}"))?;
    }
    Ok(permissions)
}

/// Parameter permissions of the direct call targets in `instance`'s MIR, by
/// callee symbol. Calls the converter synthesizes (drop glue, lang items)
/// have no entry and therefore no permission.
pub fn callee_permissions<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
) -> Result<BTreeMap<String, Vec<ArgPermission>>, String> {
    let mir = tcx.instance_mir(instance.def);
    let mut permissions = BTreeMap::new();
    for block in mir.basic_blocks.iter() {
        let mir::TerminatorKind::Call { func, .. } = &block.terminator().kind else {
            continue;
        };
        let function_ty = instance.instantiate_mir_and_normalize_erasing_regions(
            tcx,
            TypingEnv::fully_monomorphized(),
            ty::EarlyBinder::bind(func.ty(&mir.local_decls, tcx)),
        );
        let ty::FnDef(def_id, args) = *function_ty.kind() else {
            continue;
        };
        let Ok(Some(callee)) =
            Instance::try_resolve(tcx, TypingEnv::fully_monomorphized(), def_id, args)
        else {
            continue;
        };
        if matches!(
            callee.def,
            InstanceKind::Intrinsic(_) | InstanceKind::Virtual(..)
        ) {
            continue;
        }
        let symbol = tcx.symbol_name(callee).name.to_string();
        if permissions.contains_key(&symbol) {
            continue;
        }
        let abi = fn_abi(tcx, callee)?;
        let callee_permissions = abi_permissions(tcx, abi);
        if tcx.sess.opts.optimize != OptLevel::No {
            guard(abi, &callee_permissions)
                .map_err(|error| format!("permission guard for callee `{callee}`: {error}"))?;
        }
        if callee_permissions
            .iter()
            .any(|permission| *permission != ArgPermission::None)
        {
            permissions.insert(symbol, callee_permissions);
        }
    }
    Ok(permissions)
}

fn fn_abi<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
) -> Result<&'tcx callconv::FnAbi<'tcx, Ty<'tcx>>, String> {
    tcx.fn_abi_of_instance(
        TypingEnv::fully_monomorphized().as_query_input((instance, ty::List::empty())),
    )
    .map_err(|error| format!("no ABI for `{instance}`: {error:?}"))
}

fn abi_permissions<'tcx>(
    tcx: TyCtxt<'tcx>,
    abi: &callconv::FnAbi<'tcx, Ty<'tcx>>,
) -> Vec<ArgPermission> {
    abi.args
        .iter()
        .map(|argument| match argument.mode {
            callconv::PassMode::Direct(_) | callconv::PassMode::Pair(..) => {
                reference_permission(tcx, argument.layout.ty)
            }
            _ => ArgPermission::None,
        })
        .collect()
}

/// The `PointerKind` rustc derives for a direct reference argument, evaluated
/// as if optimizing (`layout.rs` `ty_and_layout_pointee_info_at`), and the
/// attribute choice of `arg_attrs_for_rust_scalar`.
fn reference_permission<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> ArgPermission {
    let ty::Ref(_, pointee, mutability) = *ty.kind() else {
        return ArgPermission::None;
    };
    let typing_env = TypingEnv::fully_monomorphized();
    let Ok(layout) = tcx.layout_of(typing_env.as_query_input(pointee)) else {
        return ArgPermission::None;
    };
    let dereferenceable = layout.is_sized().then(|| layout.size.bytes());
    match mutability {
        ty::Mutability::Not if pointee.is_freeze(tcx, typing_env) => {
            ArgPermission::Frozen { dereferenceable }
        }
        ty::Mutability::Mut
            if tcx.sess.opts.unstable_opts.mutable_noalias && pointee.is_unpin(tcx, typing_env) =>
        {
            ArgPermission::Unique { dereferenceable }
        }
        _ => ArgPermission::None,
    }
}

fn argument_attributes(mode: &callconv::PassMode) -> Option<&callconv::ArgAttributes> {
    match mode {
        callconv::PassMode::Direct(attributes) | callconv::PassMode::Pair(attributes, _) => {
            Some(attributes)
        }
        _ => None,
    }
}

/// N5.14: every kfrc permission is implied by rustc's attributes for the same
/// argument. Returns the number of checked permissions.
fn guard<'tcx>(
    abi: &callconv::FnAbi<'tcx, Ty<'tcx>>,
    permissions: &[ArgPermission],
) -> Result<usize, String> {
    let mut checked = 0;
    for (index, (argument, permission)) in abi.args.iter().zip(permissions).enumerate() {
        let (dereferenceable, read_only) = match permission {
            ArgPermission::None => continue,
            ArgPermission::Frozen { dereferenceable } => (*dereferenceable, true),
            ArgPermission::Unique { dereferenceable } => (*dereferenceable, false),
        };
        let attributes = argument_attributes(&argument.mode)
            .ok_or_else(|| format!("argument {index} has no scalar attributes"))?;
        let implied = attributes.regular.contains(ArgAttribute::NoAlias)
            && (!read_only || attributes.regular.contains(ArgAttribute::ReadOnly))
            && dereferenceable.is_none_or(|bytes| attributes.pointee_size.bytes() == bytes);
        if !implied {
            return Err(format!(
                "argument {index} `{}`: kfrc {permission:?} is not implied by rustc {attributes:?}",
                argument.layout.ty
            ));
        }
        checked += 1;
    }
    Ok(checked)
}
