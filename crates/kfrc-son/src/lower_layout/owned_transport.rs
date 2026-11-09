//! Private representation snapshots for whole-value enum Phi transport.
//! This is deliberately not a lazy field representation: every consumer must
//! transport the complete value, and cyclic control regions are excluded.
use super::*;
use crate::analysis::uses::{UseKind, Uses};

#[derive(Clone)]
pub(super) struct OwnedPhiInit {
    pub region: NodeId,
    pub slot: usize,
    pub source: Value,
    pub destination: NodeId,
}

fn scalar_leaves(graph: &FunctionGraph, ty: TyId) -> Option<u64> {
    let layout = graph.oracle.size_align(ty)?;
    if layout.size == 0 {
        return Some(0);
    }
    if graph.oracle.semantic().maybe_uninit_inner(ty).is_some() {
        return Some(layout.size.div_ceil(8));
    }
    if graph.oracle.semantic().is_enum(ty) {
        let count = graph.oracle.semantic().variant_count(ty)?;
        let mut leaves = u64::from(graph.oracle.enums.get(&ty)?.tag.is_some());
        for variant in 0..count {
            for field in graph
                .oracle
                .semantic()
                .field_types(ty, Some(variant as u32))?
            {
                leaves = leaves.checked_add(scalar_leaves(graph, field)?)?;
            }
        }
        Some(leaves)
    } else if let TypeShape::Array { element, length } = graph.oracle.shape(ty) {
        scalar_leaves(graph, *element)?.checked_mul(*length)
    } else if let Some(fields) = graph.oracle.semantic().field_types(ty, None) {
        fields.into_iter().try_fold(0u64, |sum, field| {
            sum.checked_add(scalar_leaves(graph, field)?)
        })
    } else {
        Some(1)
    }
}

fn cyclic_region(graph: &FunctionGraph, region: NodeId) -> bool {
    let mut pending = graph.inputs(region).to_vec();
    let mut seen = HashSet::new();
    while let Some(node) = pending.pop() {
        let node = graph.resolve(node);
        if node == region {
            return true;
        }
        if node.is_none() || !seen.insert(node) {
            continue;
        }
        let control = graph.control(node);
        if !control.is_none() {
            pending.push(control);
        }
        match graph.op(node) {
            Opcode::Region => pending.extend(graph.inputs(node)),
            Opcode::Projection => pending.push(graph.input(node, 0)),
            _ => {}
        }
    }
    false
}

fn sink(graph: &FunctionGraph, user: NodeId, kind: UseKind) -> bool {
    matches!(
        (graph.op(user), kind),
        (Opcode::Return, UseKind::Operand(1)) | (Opcode::StoreValue, UseKind::Operand(2))
    )
}

pub(super) fn select(graph: &FunctionGraph, demands: &HashMap<NodeId, Demand>) -> HashSet<NodeId> {
    let uses = Uses::new(graph);
    let mut candidates: HashSet<_> = graph
        .ids()
        .filter(|&node| {
            if graph.op(node) != Opcode::Phi || demands.get(&node) != Some(&Demand::All) {
                return false;
            }
            let Some(ty) = graph.rust_type(node) else {
                return false;
            };
            if !graph.oracle.semantic().is_enum(ty) {
                return false;
            }
            let Some(layout) = graph.oracle.size_align(ty) else {
                return false;
            };
            // Enum alternatives overlap physically. Retain storage only when the
            // full logical leaf representation is larger than that physical word
            // representation. This is an explicit representation-cost hypothesis,
            // not a claim that a memcpy is cheaper than every register move.
            scalar_leaves(graph, ty).is_some_and(|leaves| leaves > layout.size.div_ceil(8))
                && !cyclic_region(graph, graph.input(node, 0))
        })
        .collect();
    loop {
        let removed: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|&node| {
                uses.of(node).is_empty()
                    || uses.of(node).iter().any(|use_| {
                        !sink(graph, use_.user, use_.kind)
                            && !(matches!(use_.kind, UseKind::Operand(index) if index > 0)
                                && candidates.contains(&use_.user))
                    })
                    || graph.inputs(node)[1..].iter().any(|&input| {
                        let input = graph.resolve(input);
                        graph.op(input) != Opcode::MakeVariant && !candidates.contains(&input)
                    })
            })
            .collect();
        if removed.is_empty() {
            break;
        }
        for node in removed {
            candidates.remove(&node);
        }
    }
    candidates
}

#[derive(Default)]
struct EdgeSnapshots {
    incoming: Vec<(NodeId, usize)>,
    owned: Vec<OwnedPhiInit>,
    opaque: Vec<maybe_uninit::OpaquePhiCopy>,
}

impl LayoutLowering<'_> {
    pub(super) fn finish_snapshot_phis(&mut self) -> Result<(), LayoutError> {
        let mut edges: BTreeMap<NodeId, EdgeSnapshots> = BTreeMap::new();
        for init in std::mem::take(&mut self.owned_phis) {
            let jump = self
                .graph
                .resolve(self.graph.input(init.region, init.slot - 1));
            let edge = edges.entry(jump).or_default();
            edge.incoming.push((init.region, init.slot));
            edge.owned.push(init);
        }
        for copy in std::mem::take(&mut self.opaque_phis) {
            let jump = self
                .graph
                .resolve(self.graph.input(copy.region, copy.slot - 1));
            let edge = edges.entry(jump).or_default();
            edge.incoming.push((copy.region, copy.slot));
            edge.opaque.push(copy);
        }
        let mut effects = HashMap::new();
        for (jump, edge) in edges {
            let mut control = self.graph.resolve(self.graph.control(jump));
            let mut effect = self.opaque_effect_at(control, &mut effects)?;
            // Each owned destination is unique to this constructor input and is
            // initialized at most once: select excludes every CFG cycle through
            // the destination Region, including cycles from an enclosing loop.
            // Its address is subsequently used only by pointer Phis and copy
            // sources. No later operation writes this immutable snapshot.
            for init in edge.owned {
                self.encode_fields(&init.source, init.destination, &mut control, &mut effect)?;
            }
            // Typed constructors may read old raw snapshots on this same edge.
            // Capture them above, and stage every raw source below, before any
            // MaybeUninit Phi destination is overwritten.
            let mut staged = Vec::new();
            for copy in edge.opaque {
                let stage = self.opaque_slot(copy.ty)?;
                let size = self.graph.oracle.size_align(copy.ty).unwrap().size;
                self.raw_copy(copy.source, stage, size, &mut control, &mut effect);
                staged.push((stage, copy.destination, size));
            }
            for (stage, destination, size) in staged {
                self.raw_copy(stage, destination, size, &mut control, &mut effect);
            }
            self.graph.set_control(jump, control);
            for (region, slot) in edge.incoming {
                let effect_phi = self.opaque_effect_at(region, &mut effects)?;
                self.graph.set_input(effect_phi, slot, effect);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "owned_transport_tests.rs"]
mod tests;
