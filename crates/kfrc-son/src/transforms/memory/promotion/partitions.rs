//! Promote independently accessed logical subobjects of a retained allocation.
//!
//! Candidates come from actual typed accesses, not a type's complete field list.
//! Every other access must be either a defining enclosing store or a proven
//! disjoint byte range. Unknown offsets/counts can reach sibling fields, so
//! their source types never establish containment. Lifetime operations remain
//! in memory and reset every promoted partition of the allocation.
use super::*;
use crate::analysis::memory::{MemoryLocation, MemoryLocations};
use crate::analysis::value_evaluation::{EvaluationStats, TotalEvaluation, complete_scalar_layout};
use kfrc_body::TypeOracle;

#[derive(Clone, Copy)]
struct Access {
    node: NodeId,
    address: NodeId,
}

pub(super) struct Partitions {
    accesses: HashMap<NodeId, Vec<Access>>,
    locations: MemoryLocations,
    evaluation: TotalEvaluation,
    evaluation_stats: EvaluationStats,
    field_layouts: HashMap<(TyId, u32), bool>,
    remaining: usize,
    visits: usize,
    exhausted: bool,
}

impl Partitions {
    pub(super) fn new(
        graph: &FunctionGraph,
        cfg: &ControlFlow,
        facts: MemoryFacts,
        whole_loads: &[NodeId],
    ) -> Self {
        let mut accesses: HashMap<NodeId, Vec<Access>> = HashMap::default();
        for &node in &cfg.order {
            let pointers = match graph.op(node) {
                Opcode::LoadValue
                | Opcode::StoreValue
                | Opcode::Load
                | Opcode::Store
                | Opcode::SetVariant
                | Opcode::SlotLive
                | Opcode::SlotDead => &graph.inputs(node)[1..2],
                Opcode::CopyNonOverlapping => &graph.inputs(node)[1..3],
                _ => continue,
            };
            for &address in pointers {
                if let Some(root) = facts.root(address) {
                    accesses
                        .entry(root)
                        .or_default()
                        .push(Access { node, address });
                }
            }
        }
        let mut evaluation = TotalEvaluation::checking_phi_inputs(graph);
        for &load in whole_loads {
            evaluation.exclude_producer(graph, load);
        }
        Self {
            accesses,
            locations: MemoryLocations::from_facts(graph, facts),
            evaluation,
            evaluation_stats: EvaluationStats::default(),
            field_layouts: HashMap::default(),
            remaining: graph.len().saturating_mul(16).clamp(256, 131_072),
            visits: cfg.order.len(),
            exhausted: false,
        }
    }

    fn charge(&mut self) -> Option<()> {
        if self.remaining == 0 {
            self.exhausted = true;
            return None;
        }
        self.remaining -= 1;
        self.visits += 1;
        Some(())
    }

    pub(super) fn account(self, stats: &mut PromotionStats) {
        let visits = self.visits + self.locations.node_visits + self.evaluation_stats.node_visits;
        stats.node_visits += visits;
        stats.partition_node_visits += visits;
        stats.partition_budget_bailouts += usize::from(self.exhausted)
            + self.locations.budget_bailouts
            + self.evaluation_stats.budget_bailouts;
    }

    pub(super) fn collect(
        &mut self,
        graph: &FunctionGraph,
        uses: &Uses,
        root: NodeId,
    ) -> Vec<Slot> {
        let Some(accesses) = self.accesses.get(&root).cloned() else {
            return Vec::new();
        };
        let mut paths = HashMap::default();
        let mut candidates = HashMap::default();
        for access in &accesses {
            if !matches!(
                graph.op(access.node),
                Opcode::LoadValue | Opcode::StoreValue
            ) {
                continue;
            }
            if paths.contains_key(&access.address) {
                continue;
            }
            let path = self.path(graph, access.address, root);
            if self.exhausted {
                return Vec::new();
            }
            if let Some(path) = &path {
                let mut prefix = Vec::new();
                for (field, address) in path {
                    if self.charge().is_none() {
                        return Vec::new();
                    }
                    prefix.push(field.clone());
                    candidates.entry(prefix.clone()).or_insert(*address);
                }
            }
            paths.insert(
                access.address,
                path.map(|path| path.into_iter().map(|(field, _)| field).collect::<Vec<_>>()),
            );
        }
        let mut candidates: Vec<_> = candidates.into_iter().collect();
        candidates.sort_unstable_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        let mut selected: Vec<Vec<Field>> = Vec::new();
        let mut slots = Vec::new();
        for (path, address) in candidates {
            if selected.iter().any(|parent| path.starts_with(parent)) {
                continue;
            }
            if let Some(slot) = self.plan(graph, uses, root, address, &path, &paths, &accesses) {
                selected.push(path);
                slots.push(slot);
            }
            if self.exhausted {
                break;
            }
        }
        slots
    }

    fn path(
        &mut self,
        graph: &FunctionGraph,
        mut address: NodeId,
        root: NodeId,
    ) -> Option<Vec<(Field, NodeId)>> {
        let mut path = Vec::new();
        for _ in 0..64 {
            self.charge()?;
            if address == root {
                path.reverse();
                return Some(path);
            }
            let base = graph.resolve(graph.input(address, 0));
            let container = pointee(graph, base)?;
            let semantic = graph.oracle.semantic();
            let index = match graph.op(address) {
                Opcode::PointerCast if pointee(graph, address) == Some(container) => {
                    address = base;
                    continue;
                }
                Opcode::FieldAddr
                    if !semantic.is_enum(container) && !semantic.is_union(container) =>
                {
                    graph.payload(address)
                }
                Opcode::IndexAddr => {
                    let kfrc_body::TypeShape::Array { element, length } = semantic.shape(container)
                    else {
                        return None;
                    };
                    let index = constant_index(graph, graph.input(address, 1))?;
                    if u64::from(index) >= *length || pointee(graph, address) != Some(*element) {
                        return None;
                    }
                    index
                }
                _ => return None,
            };
            let ty = semantic.field_type(container, None, index)?;
            path.push((
                Field {
                    variant: None,
                    index,
                    ty,
                },
                address,
            ));
            address = base;
        }
        None
    }

    fn plan(
        &mut self,
        graph: &FunctionGraph,
        uses: &Uses,
        root: NodeId,
        address: NodeId,
        path: &[Field],
        paths: &HashMap<NodeId, Option<Vec<Field>>>,
        accesses: &[Access],
    ) -> Option<Slot> {
        let location = self.locations.location(graph, address)?;
        if location.size == 0 {
            return None;
        }
        let ty = pointee(graph, address)?;
        if !promotable_type(graph, ty) || !self.nonoverlapping_path(graph, root, path) {
            return None;
        }
        let mut slot = Slot {
            ty,
            accesses: HashMap::default(),
            definitions: HashMap::default(),
            preserve_storage: true,
        };
        for access in accesses {
            self.charge()?;
            let node = access.node;
            match graph.op(node) {
                Opcode::SlotLive | Opcode::SlotDead => {
                    slot.accesses.insert(node, Vec::new());
                }
                Opcode::LoadValue | Opcode::StoreValue => {
                    if let Some(Some(other)) = paths.get(&access.address) {
                        if other.starts_with(path) {
                            if !self.safe_access(graph, root, *access) {
                                return None;
                            }
                            let maximum = u32::from(graph.op(node) == Opcode::LoadValue) + 1;
                            if uses.of(node).iter().any(|usage| {
                                usage.kind() != UseKind::Operand(0)
                                    || graph.op(usage.user) != Opcode::Projection
                                    || graph.payload(usage.user) > maximum
                            }) {
                                return None;
                            }
                            slot.accesses.insert(node, other[path.len()..].to_vec());
                            continue;
                        }
                        if path.starts_with(other) {
                            if graph.op(node) != Opcode::StoreValue {
                                return None;
                            }
                            slot.definitions
                                .insert(node, (graph.input(node, 2), path[other.len()..].to_vec()));
                            continue;
                        }
                    }
                    let other = self.locations.location(graph, access.address)?;
                    if !disjoint(location, other, other.size) {
                        return None;
                    }
                }
                Opcode::CopyNonOverlapping | Opcode::SetVariant => {
                    let other = self.locations.location(graph, access.address)?;
                    let size = if graph.op(node) == Opcode::CopyNonOverlapping {
                        let count = graph.resolve(graph.input(node, 3));
                        if graph.op(count) != Opcode::Const {
                            return None;
                        }
                        let count = u64::try_from(graph.constant_value(count)).ok()?;
                        other.size.checked_mul(count)?
                    } else {
                        other.size
                    };
                    if !disjoint(location, other, size) {
                        return None;
                    }
                }
                // Raw Load/Store widths need a separate access-width contract.
                // An unknown access is never confined by its pointer's type.
                _ => return None,
            }
        }
        slot.accesses
            .keys()
            .any(|&node| matches!(graph.op(node), Opcode::LoadValue | Opcode::StoreValue))
            .then_some(slot)
    }

    fn safe_access(&mut self, graph: &FunctionGraph, root: NodeId, access: Access) -> bool {
        let Some(ty) = pointee(graph, access.address) else {
            return false;
        };
        // A removed load must not suppress representation decoding errors.
        // Do not infer char, reference or nominal validity from a typed write.
        if !promotable_type(graph, ty) {
            return false;
        }
        let Some(place) = self.locations.location(graph, access.address) else {
            return false;
        };
        let Some(layout) = pointee(graph, root).and_then(|ty| graph.oracle.size_align(ty)) else {
            return false;
        };
        if place.root != root
            || layout.align < place.align
            || place.offset % place.align != 0
            || place
                .offset
                .checked_add(place.size)
                .is_none_or(|end| end > layout.size)
        {
            return false;
        }
        if graph.op(access.node) == Opcode::StoreValue {
            self.evaluation.operands_and_encoding(
                graph,
                access.node,
                &mut self.remaining,
                &mut self.evaluation_stats,
            )
        } else {
            self.evaluation.is_total(
                graph,
                access.address,
                &mut self.remaining,
                &mut self.evaluation_stats,
            )
        }
    }

    fn nonoverlapping_path(&mut self, graph: &FunctionGraph, root: NodeId, path: &[Field]) -> bool {
        let Some(mut container) = pointee(graph, root) else {
            return false;
        };
        for field in path {
            let key = (container, field.index);
            let safe = if let Some(&safe) = self.field_layouts.get(&key) {
                safe
            } else {
                let safe = self.nonoverlapping_field(graph, container, field.index);
                self.field_layouts.insert(key, safe);
                safe
            };
            if !safe {
                return false;
            }
            container = field.ty;
        }
        true
    }

    /// Logical products are not themselves a physical non-overlap proof.
    /// Check only fields on observed paths; arrays use their regular stride.
    fn nonoverlapping_field(&mut self, graph: &FunctionGraph, ty: TyId, index: u32) -> bool {
        let Some(()) = self.charge() else {
            return false;
        };
        let Some(layout) = graph.oracle.size_align(ty) else {
            return false;
        };
        if let kfrc_body::TypeShape::Array { element, length } = graph.oracle.shape(ty) {
            return graph.oracle.size_align(*element).is_some_and(|element| {
                u64::from(index) < *length
                    && element
                        .size
                        .checked_mul(*length)
                        .is_some_and(|size| size <= layout.size)
            });
        }
        let Some(selected) = graph.oracle.field(ty, None, index) else {
            return false;
        };
        let Some(size) = graph
            .oracle
            .size_align(selected.ty)
            .map(|layout| layout.size)
        else {
            return false;
        };
        let Some(end) = selected
            .offset
            .checked_add(size)
            .filter(|&end| end <= layout.size)
        else {
            return false;
        };
        let Some(fields) = graph.oracle.semantic().field_count(ty, None) else {
            return false;
        };
        for other in 0..fields {
            if self.charge().is_none() {
                return false;
            }
            if other == u64::from(index) {
                continue;
            }
            let Some(other) = u32::try_from(other)
                .ok()
                .and_then(|i| graph.oracle.field(ty, None, i))
            else {
                return false;
            };
            let Some(other_size) = graph.oracle.size_align(other.ty).map(|layout| layout.size)
            else {
                return false;
            };
            if other_size != 0
                && other.offset < end
                && other
                    .offset
                    .checked_add(other_size)
                    .is_none_or(|other_end| selected.offset < other_end)
            {
                return false;
            }
        }
        true
    }
}

fn promotable_type(graph: &FunctionGraph, ty: TyId) -> bool {
    graph.oracle.scalar_validity(ty).is_none()
        && complete_scalar_layout(graph, ty)
        && matches!(
            graph.oracle.semantic().shape(ty),
            kfrc_body::TypeShape::Int { .. }
                | kfrc_body::TypeShape::Unit
                | kfrc_body::TypeShape::RawPointer { .. }
        )
}

fn disjoint(a: MemoryLocation, b: MemoryLocation, b_size: u64) -> bool {
    a.root == b.root
        && (a
            .offset
            .checked_add(a.size)
            .is_some_and(|end| end <= b.offset)
            || b.offset
                .checked_add(b_size)
                .is_some_and(|end| end <= a.offset))
}
