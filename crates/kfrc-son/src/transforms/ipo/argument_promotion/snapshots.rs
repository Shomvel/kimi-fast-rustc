//! Additional snapshot proofs, confined to an authenticated parameter's call.
use super::plan::Part;
use crate::analysis::memory::MemoryLocations;
use crate::{ArgPermission, FunctionGraph, NodeId, Opcode};
use kfrc_body::{IntegerWidth, Mutability, PassMode, TyId, TypeOracle, TypeShape};

pub(super) fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var("KFRC_PROTECTED_ARGUMENT_PROMOTION").as_deref() != Ok("0"))
}

/// The planner's exhaustive root-use walk permits only reads and pure address
/// projections. Frozen storage cannot change; Unique storage can only be
/// accessed through that root and its derivatives during this invocation.
/// Thus unrelated writes/calls cannot change the snapshot. No conclusion is
/// drawn from reference type or different SSA identities alone.
pub(super) fn protected_carrier(graph: &FunctionGraph, root: NodeId, index: usize) -> bool {
    let Some(ty) = graph.rust_type(root) else {
        return false;
    };
    let TypeShape::Reference {
        pointee,
        mutability,
    } = *graph.oracle.semantic().shape(ty)
    else {
        return false;
    };
    let bytes = match (graph.arg_permission(index), mutability) {
        (ArgPermission::Frozen { dereferenceable }, Mutability::Not)
        | (ArgPermission::Unique { dereferenceable }, Mutability::Mut) => dereferenceable,
        _ => None,
    };
    if graph.oracle.semantic().is_unsized(pointee)
        || graph.ids().any(|node| {
            matches!(
                graph.op(node),
                Opcode::Opaque | Opcode::CallIndirect | Opcode::ThreadLocalAddress
            )
        })
    {
        return false;
    }
    graph
        .oracle
        .size_align(pointee)
        .is_some_and(|layout| layout.size != 0 && Some(layout.size) == bytes)
}

pub(super) fn valid_accesses<'a>(
    graph: &FunctionGraph,
    root: NodeId,
    parts: impl Iterator<Item = &'a Part>,
) -> bool {
    let Some(pointee) = graph
        .rust_type(root)
        .and_then(|ty| graph.oracle.semantic().thin_pointee(ty))
    else {
        return false;
    };
    let Some(carrier) = graph.oracle.size_align(pointee) else {
        return false;
    };
    let mut locations = MemoryLocations::new(graph);
    parts.flat_map(|part| &part.loads).all(|&load| {
        locations
            .location(graph, graph.input(load, 1))
            .is_some_and(|place| {
                place.root == root
                    && place.size != 0
                    && carrier.align >= place.align
                    && place.offset % place.align == 0
                    && place
                        .offset
                        .checked_add(place.size)
                        .is_some_and(|end| end <= carrier.size)
            })
    })
}

/// Copy the complete pointer value, without adding a borrow permission to its
/// new parameter. Wide shared str/slice references use their existing two-lane
/// value representation; aggregate, vtable and cast transports stay excluded.
pub(super) fn transport(graph: &FunctionGraph, ty: TyId) -> Option<PassMode> {
    if !enabled() || graph.oracle.semantic().maybe_uninit_inner(ty).is_some() {
        return None;
    }
    match *graph.oracle.semantic().shape(ty) {
        TypeShape::Reference {
            pointee,
            mutability: Mutability::Not,
        } if matches!(
            graph.oracle.semantic().shape(pointee),
            TypeShape::Str | TypeShape::Slice { .. }
        ) =>
        {
            let semantic = graph.oracle.semantic();
            let fields = semantic.field_types(ty, None)?;
            let [data, metadata] = fields.as_slice() else {
                return None;
            };
            if !matches!(semantic.shape(*data), TypeShape::RawPointer { .. })
                || semantic.thin_pointee(*data).is_none()
                || !matches!(
                    semantic.shape(*metadata),
                    TypeShape::Int {
                        width: IntegerWidth::Pointer,
                        signed: false
                    }
                )
            {
                return None;
            }
            for (field, expected) in [0, 8].into_iter().enumerate() {
                let layout = graph.oracle.field(ty, None, field as u32)?;
                let lane = graph.oracle.size_align(fields[field])?;
                if layout.offset != expected || lane.size != 8 || lane.align != 8 {
                    return None;
                }
            }
            graph
                .oracle
                .size_align(ty)
                .filter(|layout| layout.size == 16 && layout.align == 8)
                .map(|_| PassMode::Pair)
        }
        TypeShape::Reference {
            mutability: Mutability::Not,
            ..
        }
        | TypeShape::RawPointer { .. }
            if graph.oracle.semantic().thin_pointee(ty).is_some() =>
        {
            graph
                .oracle
                .size_align(ty)
                .filter(|layout| layout.size == 8 && layout.align == 8)
                .map(|_| PassMode::Direct)
        }
        _ => None,
    }
}
