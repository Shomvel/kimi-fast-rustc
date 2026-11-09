//! Demand-driven SSA promotion for unescaped, logically addressed local storage.
//!
//! W05/F08: an inlined argument slot may become promotable on a later seeded
//! invocation. W09/F45: dynamic indexing remains storage. W10/F49/F50: escaping
//! and merged pointer roots never acquire a new alias assumption here.
//! Whole enum values can be replaced and read through variant payload paths.
//! Payload writes and tag mutations stay in storage; no partial enum update
//! operation is invented by this pass.

use kfrc_body::fx_hash::{HashMap, HashSet};
use std::collections::VecDeque;

use kfrc_body::TyId;

use crate::analysis::cfg::ControlFlow;
use crate::analysis::memory::MemoryFacts;
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};

#[derive(Clone, Debug, Default)]
pub struct PromotionStats {
    pub slots_promoted: usize,
    pub loads_removed: usize,
    pub stores_removed: usize,
    pub lifetime_ops_removed: usize,
    pub value_phis: usize,
    pub node_visits: usize,
    pub edge_visits: usize,
    pub dirty_nodes: Vec<NodeId>,
}

#[derive(Clone)]
struct Field {
    variant: Option<u32>,
    index: u32,
    ty: TyId,
}

struct Slot {
    ty: TyId,
    accesses: HashMap<NodeId, Vec<Field>>,
}

/// `candidates` contains source local identities, not NodeIds. Recompute escape
/// facts after inlining; an old escaped fact must not block a now-local slot.
pub fn run(graph: &mut FunctionGraph, candidates: Option<&HashSet<u32>>) -> PromotionStats {
    let mut stats = PromotionStats::default();
    if !matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized)
        || candidates.is_some_and(HashSet::is_empty)
    {
        return stats;
    }
    graph.resolve_edges();
    let facts = MemoryFacts::new(graph);
    stats.node_visits += facts.node_visits;
    stats.edge_visits += facts.edge_visits;
    let mut roots: Vec<_> = facts
        .local_roots()
        .filter(|(local, root)| {
            graph.op(*root) == Opcode::LocalSlot
                && facts.is_unescaped_local(*local)
                && candidates.is_none_or(|candidates| candidates.contains(local))
        })
        .collect();
    roots.sort_unstable_by_key(|(local, _)| *local);
    if roots.is_empty() {
        return stats;
    }
    let uses = Uses::new(graph);
    let cfg = ControlFlow::new(graph);
    stats.node_visits += 2 * graph.ids().count();
    stats.edge_visits += graph
        .ids()
        .map(|node| graph.inputs(node).len() + usize::from(!graph.control(node).is_none()))
        .sum::<usize>();
    // A verified control graph has explicit Regions for every merge. Refuse
    // unusual skeletons before introducing any Phi or retiring any operation.
    if cfg.order.iter().enumerate().any(|(index, node)| {
        *node != graph.start
            && graph.op(*node) != Opcode::Region
            && cfg.predecessors[index].len() != 1
    }) {
        return stats;
    }
    let mut rewrites = Vec::new();
    for (_, root) in roots {
        let Some(slot) = collect_slot(graph, &uses, &cfg, root, &mut stats) else {
            continue;
        };
        if slot.accesses.is_empty() || !accesses_are_live(graph, &cfg, &slot, &mut stats) {
            continue;
        }
        let undefined = graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(slot.ty),
        );
        stats.dirty_nodes.push(undefined);
        let mut state = States {
            graph,
            cfg: &cfg,
            slot: &slot,
            values: HashMap::default(),
            undefined,
            stats: &mut stats,
        };
        let mut accesses: Vec<_> = slot.accesses.keys().copied().collect();
        accesses.sort_unstable();
        for access in accesses {
            let replacement = if state.graph.op(access) == Opcode::LoadValue {
                let control = state.graph.control(access);
                let current = state.read(control);
                let value = state.select(current, &slot.accesses[&access]);
                Some(value)
            } else {
                None
            };
            rewrites.push((access, replacement));
        }
        state.stats.slots_promoted += 1;
    }
    // Build every slot's values before mutating the shared control skeleton.
    // Other promoted slots then still see the same CFG snapshot.
    for (access, value) in rewrites {
        let control = graph.control(access);
        let effect = graph.input(access, 0);
        for usage in uses.of(access) {
            let projection = usage.user;
            let replacement = match graph.payload(projection) {
                0 => control,
                1 => effect,
                2 => value.expect("load value projection"),
                _ => unreachable!("tuple uses checked before mutation"),
            };
            stats
                .dirty_nodes
                .extend(uses.of(projection).iter().map(|usage| usage.user));
            graph.replace(projection, replacement);
        }
        match graph.op(access) {
            Opcode::LoadValue => stats.loads_removed += 1,
            Opcode::StoreValue => stats.stores_removed += 1,
            Opcode::SlotLive | Opcode::SlotDead => stats.lifetime_ops_removed += 1,
            _ => unreachable!("only promotable accesses are collected"),
        }
        graph.remove_node(access);
    }
    graph.resolve_edges();
    stats.dirty_nodes.sort_unstable();
    stats.dirty_nodes.dedup();
    stats
}

fn pointee(graph: &FunctionGraph, node: NodeId) -> Option<TyId> {
    let ValueType::Rust(ty) = graph.value_type(node) else {
        return None;
    };
    graph.oracle.semantic().thin_pointee(*ty)
}

fn collect_slot(
    graph: &FunctionGraph,
    uses: &Uses,
    cfg: &ControlFlow,
    root: NodeId,
    stats: &mut PromotionStats,
) -> Option<Slot> {
    let ty = pointee(graph, root)?;
    if graph.oracle.semantic().is_union(ty) {
        return None;
    }
    let mut slot = Slot {
        ty,
        accesses: HashMap::default(),
    };
    let mut pending = VecDeque::from([(root, Vec::<Field>::new())]);
    let mut seen = HashSet::default();
    while let Some((address, path)) = pending.pop_front() {
        if !seen.insert(address) {
            continue;
        }
        stats.node_visits += 1;
        for usage in uses.of(address) {
            stats.edge_visits += 1;
            let user = usage.user;
            match (graph.op(user), usage.kind) {
                (Opcode::FieldAddr, UseKind::Operand(0)) => {
                    let container = pointee(graph, address)?;
                    if graph.oracle.semantic().is_enum(container)
                        || graph.oracle.semantic().is_union(container)
                    {
                        return None;
                    }
                    let field = graph.payload(user);
                    let field_ty = graph.oracle.semantic().field_type(container, None, field)?;
                    let mut path = path.clone();
                    path.push(Field {
                        variant: None,
                        index: field,
                        ty: field_ty,
                    });
                    pending.push_back((user, path));
                }
                (Opcode::VariantFieldAddr, UseKind::Operand(0)) => {
                    let container = pointee(graph, address)?;
                    let (variant, field) =
                        *graph.variant_fields.get(graph.payload(user) as usize)?;
                    let field_ty =
                        graph
                            .oracle
                            .semantic()
                            .field_type(container, Some(variant), field)?;
                    let mut path = path.clone();
                    path.push(Field {
                        variant: Some(variant),
                        index: field,
                        ty: field_ty,
                    });
                    pending.push_back((user, path));
                }
                (Opcode::PointerCast, UseKind::Operand(0))
                    if pointee(graph, address) == pointee(graph, user) =>
                {
                    pending.push_back((user, path.clone()));
                }
                (Opcode::RequireDefined, UseKind::Operand(1)) => {
                    // The requirement is a noncapturing sink, not an address
                    // transformation. Keep its execution point after promotion.
                }

                (
                    Opcode::LoadValue | Opcode::StoreValue | Opcode::SlotLive | Opcode::SlotDead,
                    UseKind::Operand(1),
                ) => {
                    // There is no InsertVariantField contract. Even a write
                    // farther down a product payload must keep the whole
                    // allocation in storage, including its other accesses.
                    if graph.op(user) == Opcode::StoreValue
                        && path.iter().any(|field| field.variant.is_some())
                    {
                        return None;
                    }
                    if !cfg.contains(user) {
                        return None;
                    }
                    let max_projection = if graph.op(user) == Opcode::LoadValue {
                        2
                    } else {
                        1
                    };
                    if uses.of(user).iter().any(|usage| {
                        usage.kind != UseKind::Operand(0)
                            || graph.op(usage.user) != Opcode::Projection
                            || graph.payload(usage.user) > max_projection
                    }) {
                        return None;
                    }
                    slot.accesses.insert(user, path.clone());
                }
                // Copy, indexing, tag mutation, raw loads and every value
                // use of the address require storage, even when unescaped.
                _ => return None,
            }
        }
    }
    Some(slot)
}

/// Every path leaving SlotDead must encounter SlotLive before a memory access.
/// Starting at kills (rather than scanning each slot over the whole graph) also
/// handles loops and branch joins; no dynamic path is assumed infeasible here.
fn accesses_are_live(
    graph: &FunctionGraph,
    cfg: &ControlFlow,
    slot: &Slot,
    stats: &mut PromotionStats,
) -> bool {
    let mut pending: Vec<_> = slot
        .accesses
        .keys()
        .filter(|node| graph.op(**node) == Opcode::SlotDead)
        .flat_map(|node| cfg.successors(*node).iter().copied())
        .collect();
    let mut seen = HashSet::default();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            continue;
        }
        stats.node_visits += 1;
        if slot.accesses.contains_key(&node) {
            match graph.op(node) {
                Opcode::SlotLive => continue,
                Opcode::LoadValue | Opcode::StoreValue => return false,
                _ => {}
            }
        }
        stats.edge_visits += cfg.successors(node).len();
        pending.extend_from_slice(cfg.successors(node));
    }
    true
}

struct States<'a> {
    graph: &'a mut FunctionGraph,
    cfg: &'a ControlFlow,
    slot: &'a Slot,
    values: HashMap<NodeId, NodeId>,
    undefined: NodeId,
    stats: &'a mut PromotionStats,
}

enum Work {
    Read(NodeId),
    Forward(NodeId, NodeId),
    Insert(NodeId, NodeId),
    Phi(NodeId, NodeId),
}

impl States<'_> {
    /// Iterative lazy SSA construction: Region placeholders break backedges,
    /// and whole-value writes stop the backward walk immediately.
    fn read(&mut self, node: NodeId) -> NodeId {
        let mut pending = vec![Work::Read(node)];
        while let Some(work) = pending.pop() {
            match work {
                Work::Read(node) => {
                    if self.values.contains_key(&node) {
                        continue;
                    }
                    self.stats.node_visits += 1;
                    if node == self.graph.start {
                        self.values.insert(node, self.undefined);
                    } else if self.graph.op(node) == Opcode::Region {
                        let predecessors = self.graph.inputs(node).to_vec();
                        let phi = self.graph.add_growable_node(
                            Opcode::Phi,
                            NodeId::NONE,
                            predecessors.len() + 1,
                            0,
                            ValueType::Rust(self.slot.ty),
                        );
                        self.graph.set_input(phi, 0, node);
                        self.values.insert(node, phi);
                        self.stats.value_phis += 1;
                        self.stats.dirty_nodes.push(phi);
                        pending.push(Work::Phi(node, phi));
                        self.stats.edge_visits += predecessors.len();
                        pending.extend(predecessors.into_iter().map(Work::Read));
                    } else if let Some(path) = self.slot.accesses.get(&node)
                        && matches!(
                            self.graph.op(node),
                            Opcode::SlotLive | Opcode::SlotDead | Opcode::StoreValue
                        )
                    {
                        match self.graph.op(node) {
                            Opcode::SlotLive | Opcode::SlotDead => {
                                self.values.insert(node, self.undefined);
                            }
                            Opcode::StoreValue if path.is_empty() => {
                                self.values.insert(node, self.graph.input(node, 2));
                            }
                            Opcode::StoreValue => {
                                let before = self.graph.control(node);
                                pending.push(Work::Insert(node, before));
                                pending.push(Work::Read(before));
                                self.stats.edge_visits += 1;
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        let index = self.cfg.position[&node];
                        let before = self.cfg.order[self.cfg.predecessors[index][0]];
                        pending.push(Work::Forward(node, before));
                        pending.push(Work::Read(before));
                        self.stats.edge_visits += 1;
                    }
                }
                Work::Forward(node, before) => {
                    self.values.insert(node, self.values[&before]);
                }
                Work::Insert(node, before) => {
                    let value = self.insert(
                        self.values[&before],
                        self.graph.input(node, 2),
                        &self.slot.accesses[&node].clone(),
                    );
                    self.values.insert(node, value);
                }
                Work::Phi(region, phi) => {
                    let predecessors = self.graph.inputs(region).to_vec();
                    for (index, predecessor) in predecessors.into_iter().enumerate() {
                        self.graph
                            .set_input(phi, index + 1, self.values[&predecessor]);
                    }
                    if self.graph.inputs(phi).len() == 2 {
                        let value = self.graph.resolve(self.graph.input(phi, 1));
                        if value != phi {
                            self.graph.replace(phi, value);
                        }
                    }
                }
            }
        }
        self.values[&node]
    }

    fn select(&mut self, mut value: NodeId, path: &[Field]) -> NodeId {
        for field in path {
            let (opcode, payload) = match field.variant {
                Some(variant) => (
                    Opcode::VariantField,
                    self.graph.intern_variant_field(variant, field.index),
                ),
                None => (Opcode::Field, field.index),
            };
            value = self.graph.add_node(
                opcode,
                NodeId::NONE,
                &[value],
                payload,
                ValueType::Rust(field.ty),
            );
            self.stats.dirty_nodes.push(value);
        }
        value
    }

    fn insert(&mut self, old: NodeId, mut value: NodeId, path: &[Field]) -> NodeId {
        let mut containers = Vec::with_capacity(path.len());
        let mut container = old;
        let mut ty = self.slot.ty;
        for (index, field) in path.iter().enumerate() {
            containers.push((container, ty));
            if index + 1 < path.len() {
                container = self.select(container, std::slice::from_ref(field));
                ty = field.ty;
            }
        }
        for ((container, ty), field) in containers.into_iter().zip(path).rev() {
            value = self.graph.add_node(
                Opcode::InsertField,
                NodeId::NONE,
                &[container, value],
                field.index,
                ValueType::Rust(ty),
            );
            self.stats.dirty_nodes.push(value);
        }
        value
    }
}

#[cfg(test)]
#[path = "promotion_tests.rs"]
mod tests;
