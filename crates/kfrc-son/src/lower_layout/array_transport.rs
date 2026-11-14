//! Keep dense, whole arrays in private storage at their original read point.
//!
//! This first transport representation deliberately excludes element observers,
//! updates and aggregate Phis. Those continue to use scalar leaves. In
//! particular, a loop cannot carry an address to storage overwritten by the
//! next execution of its defining load.
use super::*;

fn dense_array(graph: &FunctionGraph, ty: TyId) -> bool {
    let TypeShape::Array { element, length } = graph.oracle.shape(ty) else {
        return false;
    };
    // A word-wide leaf already has the same density as block transport. Choose
    // storage only when scalar expansion creates at least 32 narrow leaves.
    *length >= 32
        && graph
            .oracle
            .size_align(*element)
            .is_some_and(|layout| layout.size > 0 && layout.size < 8)
        && matches!(
            graph.oracle.shape(*element),
            TypeShape::Int { .. } | TypeShape::Bool
        )
}

fn contains(graph: &FunctionGraph, owner: TyId, needle: TyId, seen: &mut HashSet<TyId>) -> bool {
    if owner == needle {
        return true;
    }
    if !seen.insert(owner) {
        return false;
    }
    match graph.oracle.shape(owner) {
        TypeShape::Array { element, .. } => contains(graph, *element, needle, seen),
        // Do not traverse the pointee of any pointer or reference.
        _ if graph.oracle.semantic().is_enum(owner) => graph
            .oracle
            .semantic()
            .variant_count(owner)
            .is_some_and(|count| {
                (0..count).any(|variant| {
                    graph
                        .oracle
                        .semantic()
                        .field_types(owner, Some(variant as u32))
                        .is_some_and(|fields| {
                            fields
                                .into_iter()
                                .any(|ty| contains(graph, ty, needle, seen))
                        })
                })
            }),
        _ => graph
            .oracle
            .semantic()
            .field_types(owner, None)
            .is_some_and(|fields| {
                fields
                    .into_iter()
                    .any(|ty| contains(graph, ty, needle, seen))
            }),
    }
}

pub(super) fn select(graph: &FunctionGraph) -> HashSet<TyId> {
    let candidates: HashSet<_> = graph
        .ids()
        .filter_map(|node| graph.rust_type(node))
        .filter(|&ty| dense_array(graph, ty))
        .collect();
    candidates
        .into_iter()
        .filter(|&array| {
            !graph.ids().any(|node| match graph.op(node) {
                Opcode::Field | Opcode::InsertField => {
                    graph.rust_type(graph.input(node, 0)) == Some(array)
                }
                Opcode::Phi => graph
                    .rust_type(node)
                    .is_some_and(|ty| contains(graph, ty, array, &mut HashSet::default())),
                _ => false,
            })
        })
        .collect()
}

impl LayoutLowering<'_> {
    pub(super) fn snapshot_array(
        &mut self,
        source: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<Value, LayoutError> {
        let destination = self.opaque_slot(ty)?;
        let size = self.graph.oracle.size_align(ty).unwrap().size;
        self.raw_copy(source, destination, size, control, effect);
        self.stats.transported_arrays += 1;
        Ok(Value {
            ty,
            parts: Parts::OwnedStorage(destination),
        })
    }
}
