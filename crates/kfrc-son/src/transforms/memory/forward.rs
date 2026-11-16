//! Reuse a typed snapshot from a dominating identical store or load.
//!
//! The query walks the single effect chain back from the load (RS4 D5) and
//! crosses only effect nodes that leave the loaded place unchanged:
//!
//! - every read (`LoadValue`), for any place;
//! - a direct call with a same-crate no-write proof; its execution and the
//!   original load's control/effect continuation remain in place;
//! - every effect node except `Opaque`, when the place is based on a `Frozen`
//!   parameter (Q1: `noalias readonly` covers the whole execution, callees
//!   included);
//! - a direct call whose sized `Frozen` reference covers this place (Q3: the
//!   callee's protector forbids every write to the borrowed object during the
//!   call). A protected object can cover a typed subfield; it does not cover
//!   the pointee of a pointer loaded from that object.
//!
//! An effect Phi is crossed when all of its inputs reach one memory version
//! through such nodes; inputs that return to a Phi under inspection (loop
//! back edges) leave that version unchanged. Every predecessor then holds the
//! same contents of the place, so a matching access on any of the merged
//! paths provides the value once it dominates the load. Lifetime changes,
//! requirements and other writes stop the walk for every other place.

use crate::analysis::cfg::ControlFlow;
use crate::analysis::dominators::Dominators;
use crate::analysis::memory::{MemoryLocations, is_frozen_parameter_address};
use crate::analysis::uses::{UseKind, Uses};
use crate::{ArgPermission, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::{Mutability, TyId, TypeOracle, TypeShape};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForwardStats {
    pub loads_forwarded: usize,
    /// Forwarded loads whose walk crossed a write-capable node through Q1.
    pub frozen_loads_reused: usize,
    /// Calls crossed by successful forwards (attributes, Q1 or Q3).
    pub calls_crossed_by_forwarding: usize,
    /// Calls crossed through a `Frozen` call-site argument (Q3).
    pub call_permissions_used: usize,
    /// Effect Phis crossed by successful forwards.
    pub effect_phis_crossed: usize,
    /// Writes/lifetime events proved to address separate storage or bytes.
    pub disjoint_writes_crossed: usize,
    pub location_node_visits: usize,
    pub location_budget_bailouts: usize,
    pub walk_steps: usize,
    pub budget_bailouts: usize,
}

/// Effect nodes visited per load query, shared by all paths of the query.
const WALK_BUDGET: usize = 64;

pub fn run(graph: &mut FunctionGraph) -> ForwardStats {
    let mut stats = ForwardStats::default();
    if !matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized) {
        return stats;
    }
    graph.resolve_edges();
    let uses = Uses::new(graph);
    let loads: Vec<_> = graph
        .ids()
        .filter(|node| graph.op(*node) == Opcode::LoadValue)
        .collect();
    // Forwarding only redirects projections; the control skeleton is unchanged.
    let mut dominance: Option<(ControlFlow, Dominators)> = None;
    let mut locations = None;
    for load in loads {
        let Some(available) = available_snapshot(
            graph,
            &uses,
            load,
            &mut dominance,
            &mut locations,
            &mut stats,
        ) else {
            continue;
        };
        // A verified graph only uses a tuple result through its projections.
        // Do not partially rewrite an unexpected tuple consumer.
        let consumers = uses.of(load);
        if consumers.iter().any(|usage| {
            usage.kind() != UseKind::Operand(0)
                || graph.op(usage.user) != Opcode::Projection
                || graph.payload(usage.user) > 2
        }) {
            continue;
        }
        // No node is added and each load's tuple users are left untouched until
        // that load is visited, so this precomputed use map remains sufficient.
        for projection in consumers.iter().map(|usage| usage.user) {
            let replacement = match graph.payload(projection) {
                0 => available.control,
                1 => available.effect,
                2 => available.value,
                _ => unreachable!("projection payloads checked above"),
            };
            graph.replace(projection, replacement);
        }
        graph.remove_node(load);
        stats.loads_forwarded += 1;
        let crossed = available.crossed;
        stats.frozen_loads_reused += usize::from(crossed.frozen_writers > 0);
        stats.calls_crossed_by_forwarding += crossed.calls;
        stats.call_permissions_used += crossed.call_permissions;
        stats.effect_phis_crossed += crossed.effect_phis;
        stats.disjoint_writes_crossed += crossed.disjoint_writes;
    }
    if let Some(locations) = locations {
        stats.location_node_visits = locations.node_visits;
        stats.location_budget_bailouts = locations.budget_bailouts;
    }
    graph.resolve_edges();
    stats
}

struct Available {
    control: NodeId,
    effect: NodeId,
    value: NodeId,
    crossed: Crossed,
}

/// What a successful walk crossed, for statistics only.
#[derive(Clone, Copy, Debug, Default)]
struct Crossed {
    frozen_writers: usize,
    calls: usize,
    call_permissions: usize,
    effect_phis: usize,
    disjoint_writes: usize,
}

fn available_snapshot(
    graph: &FunctionGraph,
    uses: &Uses,
    load: NodeId,
    dominance: &mut Option<(ControlFlow, Dominators)>,
    locations: &mut Option<MemoryLocations>,
    stats: &mut ForwardStats,
) -> Option<Available> {
    if graph.inputs(load).len() != 2 {
        return None;
    }
    let ValueType::Tuple(results) = graph.value_type(load) else {
        return None;
    };
    if results.len() != 3
        || results[0] != ValueType::Control
        || results[1] != ValueType::Effect
        || !matches!(results[2], ValueType::Rust(_))
    {
        return None;
    }
    let ValueType::Rust(loaded) = results[2] else {
        unreachable!("checked above")
    };
    let effect = graph.resolve(graph.input(load, 0));
    let control = graph.resolve(graph.control(load));
    let address = graph.resolve(graph.input(load, 1));
    let mut walk = Walk {
        graph,
        load,
        control,
        address,
        loaded,
        frozen: is_frozen_parameter_address(graph, address),
        dominance,
        locations,
        steps: 0,
        on_stack: Vec::new(),
    };
    let found = walk.find(effect, Crossed::default());
    stats.walk_steps += walk.steps;
    stats.budget_bailouts += usize::from(walk.steps > WALK_BUDGET);
    let (access, crossed) = found?;
    let value = if graph.op(access) == Opcode::StoreValue {
        if graph.inputs(access).len() != 3 {
            return None;
        }
        graph.resolve(graph.input(access, 2))
    } else {
        if graph.inputs(access).len() != 2 {
            return None;
        }
        // Reuse the captured owned value, not its address. Keep the first
        // load and its requirements at their original observation point.
        let projection = uses
            .of(access)
            .iter()
            .find(|usage| {
                usage.kind() == UseKind::Operand(0)
                    && graph.op(usage.user) == Opcode::Projection
                    && graph.payload(usage.user) == 2
            })?
            .user;
        graph.resolve(projection)
    };
    if graph.value_type(value) != &results[2] {
        return None;
    }
    Some(Available {
        control,
        effect,
        value,
        crossed,
    })
}

struct Walk<'a> {
    graph: &'a FunctionGraph,
    load: NodeId,
    control: NodeId,
    address: NodeId,
    loaded: TyId,
    frozen: bool,
    dominance: &'a mut Option<(ControlFlow, Dominators)>,
    locations: &'a mut Option<MemoryLocations>,
    steps: usize,
    /// Effect Phis whose inputs are being inspected.
    on_stack: Vec<NodeId>,
}

/// The memory version an effect input reaches across preserving nodes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Version {
    /// An effect that is not crossed: a write, the function entry or a Phi
    /// whose inputs disagree.
    At(NodeId),
    /// The path returns to a Phi under inspection and changes nothing.
    Unchanged,
}

impl Walk<'_> {
    fn spend(&mut self) -> bool {
        self.steps += 1;
        self.steps <= WALK_BUDGET
    }

    /// The nearest matching access on a path from `effect` back whose
    /// memory contents of the place equal those at the load.
    fn find(&mut self, effect: NodeId, mut crossed: Crossed) -> Option<(NodeId, Crossed)> {
        let graph = self.graph;
        let mut effect = graph.resolve(effect);
        loop {
            if effect.is_none() || !self.spend() {
                return None;
            }
            if graph.op(effect) == Opcode::Phi && graph.value_type(effect) == &ValueType::Effect {
                if self.on_stack.contains(&effect) {
                    return None;
                }
                // Each predecessor must hold the same contents of the place
                // before any one of them can supply the value.
                if self.version_of_phi(effect)? == Version::At(effect) {
                    return None;
                }
                crossed.effect_phis += 1;
                self.on_stack.push(effect);
                let inputs: Vec<_> = graph.inputs(effect)[1..].to_vec();
                let found = inputs
                    .into_iter()
                    .find_map(|input| self.find(input, crossed));
                self.on_stack.pop();
                return found;
            }
            let node = projection_source(graph, effect, 1)?;
            if node != self.load && self.matches(node) {
                if self.dominates_load(node) {
                    return Some((node, crossed));
                }
                // A non-dominating read on this path leaves the place as is.
                if graph.op(node) != Opcode::LoadValue {
                    return None;
                }
            } else if !self.preserves(node, &mut crossed) {
                return None;
            }
            effect = graph.resolve(*graph.inputs(node).first()?);
        }
    }

    /// The single version every input of `phi` reaches: `At(phi)` when the
    /// inputs disagree, `Unchanged` when all inputs return to Phis under
    /// inspection, `None` when the budget runs out.
    fn version_of_phi(&mut self, phi: NodeId) -> Option<Version> {
        self.on_stack.push(phi);
        let inputs: Vec<_> = self.graph.inputs(phi)[1..].to_vec();
        let mut merged = Some(Version::Unchanged);
        for input in inputs {
            let Some(version) = self.version(input) else {
                merged = None;
                break;
            };
            match (merged, version) {
                (_, Version::Unchanged) => {}
                (Some(Version::Unchanged), version) => merged = Some(version),
                (Some(previous), version) if previous == version => {}
                _ => {
                    merged = Some(Version::At(phi));
                    break;
                }
            }
        }
        self.on_stack.pop();
        merged
    }

    fn version(&mut self, effect: NodeId) -> Option<Version> {
        let graph = self.graph;
        let mut effect = graph.resolve(effect);
        loop {
            if effect.is_none() || !self.spend() {
                return None;
            }
            if graph.op(effect) == Opcode::Phi && graph.value_type(effect) == &ValueType::Effect {
                if self.on_stack.contains(&effect) {
                    return Some(Version::Unchanged);
                }
                match self.version_of_phi(effect)? {
                    Version::At(version) if version != effect => {
                        effect = version;
                        continue;
                    }
                    version => return Some(version),
                }
            }
            let Some(node) = projection_source(graph, effect, 1) else {
                return Some(Version::At(effect));
            };
            if !self.preserves(node, &mut Crossed::default()) {
                return Some(Version::At(effect));
            }
            let Some(&input) = graph.inputs(node).first() else {
                return Some(Version::At(effect));
            };
            effect = graph.resolve(input);
        }
    }

    fn matches(&mut self, node: NodeId) -> bool {
        let graph = self.graph;
        matches!(graph.op(node), Opcode::StoreValue | Opcode::LoadValue)
            && graph.inputs(node).len() >= 2
            && (same_address(graph, graph.input(node, 1), self.address)
                || self
                    .locations
                    .get_or_insert_with(|| MemoryLocations::new(graph))
                    .same_typed_place(graph, graph.input(node, 1), self.address))
    }

    /// Whether effect node `node` leaves the loaded place unchanged.
    fn preserves(&mut self, node: NodeId, crossed: &mut Crossed) -> bool {
        let graph = self.graph;
        match graph.op(node) {
            Opcode::LoadValue => true,
            Opcode::StoreValue | Opcode::SetVariant
                if self
                    .locations
                    .get_or_insert_with(|| MemoryLocations::new(graph))
                    .disjoint(graph, self.address, graph.input(node, 1)) =>
            {
                crossed.disjoint_writes += 1;
                true
            }
            Opcode::SlotLive | Opcode::SlotDead
                if self
                    .locations
                    .get_or_insert_with(|| MemoryLocations::new(graph))
                    .separate_allocations(graph, self.address, graph.input(node, 1)) =>
            {
                crossed.disjoint_writes += 1;
                true
            }
            Opcode::Call
                if graph
                    .callee_attrs(graph.payload(node))
                    .is_some_and(|attrs| attrs.memory.cannot_write()) =>
            {
                crossed.calls += 1;
                true
            }
            Opcode::Call if self.frozen => {
                crossed.calls += 1;
                crossed.frozen_writers += 1;
                true
            }
            Opcode::Call if self.call_reads_place(node) => {
                crossed.calls += 1;
                crossed.call_permissions += 1;
                true
            }
            Opcode::CallIndirect
            | Opcode::StoreValue
            | Opcode::Store
            | Opcode::SetVariant
            | Opcode::CopyNonOverlapping
            | Opcode::SlotLive
            | Opcode::SlotDead
            | Opcode::RequireDefined
            | Opcode::Assume
            | Opcode::TransmuteValue
            | Opcode::ThreadLocalAddress
            | Opcode::Load
                if self.frozen =>
            {
                crossed.calls += usize::from(graph.op(node) == Opcode::CallIndirect);
                crossed.frozen_writers += 1;
                true
            }
            _ => false,
        }
    }

    /// Q3 protects the complete sized pointee, including its subfields. Both
    /// exact-place and subfield reuse require the producer's full-size carrier;
    /// hand-built graphs can otherwise carry zero, short or overstated sizes.
    fn call_reads_place(&mut self, call: NodeId) -> bool {
        let graph = self.graph;
        let callee = graph.payload(call);
        graph.inputs(call)[1..]
            .iter()
            .enumerate()
            .any(|(argument, &value)| {
                let ArgPermission::Frozen {
                    dereferenceable: Some(bytes),
                } = graph.callee_permission(callee, argument)
                else {
                    return false;
                };
                let Some(pointee) = shared_reference_pointee(graph, value) else {
                    return false;
                };
                let Some(layout) = graph.oracle.size_align(pointee) else {
                    return false;
                };
                // Even a Unit load observes allocation/liveness in S. A
                // zero-byte protector is not an access-validity proof.
                if layout.size == 0 || bytes != layout.size {
                    return false;
                }
                let locations = self
                    .locations
                    .get_or_insert_with(|| MemoryLocations::new(graph));
                let (Some(borrowed), Some(loaded)) = (
                    locations.location(graph, value),
                    locations.location(graph, self.address),
                ) else {
                    return false;
                };
                borrowed.root == loaded.root
                    && loaded.ty == self.loaded
                    && loaded.size != 0
                    && borrowed.offset <= loaded.offset
                    && borrowed
                        .offset
                        .checked_add(borrowed.size)
                        .zip(loaded.offset.checked_add(loaded.size))
                        .is_some_and(|(end, loaded_end)| loaded_end <= end)
            })
    }

    fn dominates_load(&mut self, access: NodeId) -> bool {
        if control_descends_from_access(self.graph, self.control, access) {
            return true;
        }
        let graph = self.graph;
        let (cfg, dominators) = self.dominance.get_or_insert_with(|| {
            let cfg = ControlFlow::new(graph);
            let dominators = Dominators::new(&cfg);
            (cfg, dominators)
        });
        access != self.control && dominators.dominates(cfg, access, self.control)
    }
}

fn shared_reference_pointee(graph: &FunctionGraph, value: NodeId) -> Option<TyId> {
    let ValueType::Rust(ty) = graph.value_type(graph.resolve(value)) else {
        return None;
    };
    match graph.oracle.semantic().shape(*ty) {
        TypeShape::Reference {
            mutability: Mutability::Not,
            pointee,
        } if !graph.oracle.semantic().is_unsized(*pointee) => Some(*pointee),
        _ => None,
    }
}

/// Equal address expressions: the same node, the same-typed `PointerCast`
/// of equal addresses, the same field or index step from equal addresses,
/// or the same variant field of one enum value. A cast only retypes; it
/// keeps the address and its provenance. A variant field's witness only
/// scopes where the field may be read; the field value is fixed by the enum
/// value, so two reads of it denote one place.
fn same_address(graph: &FunctionGraph, a: NodeId, b: NodeId) -> bool {
    let (mut a, mut b) = (graph.resolve(a), graph.resolve(b));
    for _ in 0..=8 {
        if a == b {
            return true;
        }
        if a.is_none()
            || b.is_none()
            || graph.op(a) != graph.op(b)
            || graph.value_type(a) != graph.value_type(b)
        {
            return false;
        }
        match graph.op(a) {
            Opcode::PointerCast => {}
            Opcode::FieldAddr | Opcode::VariantFieldAddr | Opcode::VariantField
                if graph.payload(a) == graph.payload(b) => {}
            Opcode::IndexAddr
                if graph.resolve(graph.input(a, 1)) == graph.resolve(graph.input(b, 1)) => {}
            _ => return false,
        }
        a = graph.resolve(graph.input(a, 0));
        b = graph.resolve(graph.input(b, 0));
    }
    false
}

fn control_descends_from_access(
    graph: &FunctionGraph,
    mut control: NodeId,
    access: NodeId,
) -> bool {
    // Walking only case projections cannot cross a merge or enter a new
    // dynamic loop instance. Bound the walk even for malformed cyclic input.
    for _ in 0..=32 {
        if control.is_none()
            || graph.op(control) != Opcode::Projection
            || graph.value_type(control) != &ValueType::Control
            || graph.inputs(control).len() != 1
        {
            return false;
        }
        let source = graph.resolve(graph.input(control, 0));
        let index = graph.payload(control) as usize;
        if source == access {
            return index == 0;
        }
        if source.is_none() || !matches!(graph.op(source), Opcode::If | Opcode::VariantSwitch) {
            return false;
        }
        let ValueType::Tuple(outputs) = graph.value_type(source) else {
            return false;
        };
        if outputs.get(index) != Some(&ValueType::Control) {
            return false;
        }
        control = graph.resolve(graph.control(source));
    }
    false
}

fn projection_source(graph: &FunctionGraph, node: NodeId, index: u32) -> Option<NodeId> {
    if node.is_none() || graph.op(node) != Opcode::Projection || graph.payload(node) != index {
        return None;
    }
    Some(graph.resolve(*graph.inputs(node).first()?))
}

#[cfg(test)]
#[path = "forward_range_tests.rs"]
mod tests;
