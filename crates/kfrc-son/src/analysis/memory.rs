//! Whole-function allocation provenance and conservative escape facts.
//!
//! A root crossing a value Phi, aggregate packaging, an unknown call, a store
//! of its address or pointer-to-integer conversion escapes for the whole
//! function (W05/F46/F49/F50). No facts come from &T or &mut T alone.

use kfrc_body::fx_hash::{HashMap, HashSet};
use std::collections::VecDeque;

use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::TypeShape;

pub struct MemoryFacts {
    roots: Vec<Option<NodeId>>,
    locals: HashMap<u32, NodeId>,
    identities: HashMap<NodeId, u32>,
    escaped: HashSet<u32>,
    pub node_visits: usize,
    pub edge_visits: usize,
}

impl MemoryFacts {
    pub fn new(graph: &FunctionGraph) -> Self {
        let uses = Uses::new(graph);
        let mut facts = Self {
            roots: vec![None; graph.len()],
            locals: HashMap::default(),
            identities: HashMap::default(),
            escaped: HashSet::default(),
            node_visits: 0,
            edge_visits: 0,
        };
        let mut queue = VecDeque::new();
        for node in graph.ids() {
            facts.node_visits += 2; // Uses construction plus this provenance scan.
            facts.edge_visits +=
                graph.inputs(node).len() + usize::from(!graph.control(node).is_none());
            let local = match graph.op(node) {
                Opcode::LocalSlot => Some(graph.payload(node)),
                Opcode::StackSlot => graph.stack_slot_locals.get(&graph.payload(node)).copied(),
                _ => None,
            };
            if let Some(local) = local {
                if facts.locals.insert(local, node).is_some() {
                    facts.escaped.insert(local);
                }
                facts.identities.insert(node, local);
            }
            if local.is_some() || graph.op(node) == Opcode::SymbolAddress {
                facts.roots[node.index()] = Some(node);
                queue.push_back(node);
            }
        }
        while let Some(node) = queue.pop_front() {
            let root = facts.roots[node.index()].expect("queued provenance root");
            for usage in uses.of(node) {
                facts.edge_visits += 1;
                let user = usage.user;
                let mut derived = false;
                let safe = match usage.kind {
                    UseKind::Control => false,
                    UseKind::Operand(index) => {
                        if index == 0 && address_projection(graph, user) {
                            derived = true;
                            true
                        } else if index == 1 && graph.op(user) == Opcode::RequireDefined {
                            // This semantic requirement neither captures the
                            // address nor produces a derived pointer value.
                            true
                        } else {
                            (index == 1
                                && matches!(
                                    graph.op(user),
                                    Opcode::Load
                                        | Opcode::Store
                                        | Opcode::LoadValue
                                        | Opcode::StoreValue
                                        | Opcode::SetVariant
                                        | Opcode::SlotLive
                                        | Opcode::SlotDead
                                        | Opcode::CopyNonOverlapping
                                ))
                                || (index == 2 && graph.op(user) == Opcode::CopyNonOverlapping)
                        }
                    }
                };
                if derived && facts.roots[user.index()].is_none() {
                    facts.roots[user.index()] = Some(root);
                    queue.push_back(user);
                }
                if !safe && let Some(local) = facts.identities.get(&root) {
                    facts.escaped.insert(*local);
                }
            }
        }
        facts
    }

    pub fn root(&self, node: NodeId) -> Option<NodeId> {
        self.roots.get(node.index()).copied().flatten()
    }

    pub fn local_root(&self, local: u32) -> Option<NodeId> {
        self.locals.get(&local).copied()
    }

    pub fn local_identity(&self, root: NodeId) -> Option<u32> {
        self.identities.get(&root).copied()
    }

    pub fn is_unescaped_local(&self, local: u32) -> bool {
        self.locals.contains_key(&local) && !self.escaped.contains(&local)
    }

    pub fn local_roots(&self) -> impl Iterator<Item = (u32, NodeId)> + '_ {
        self.locals.iter().map(|(&local, &node)| (local, node))
    }
}

fn address_projection(graph: &FunctionGraph, node: NodeId) -> bool {
    matches!(
        graph.op(node),
        Opcode::FieldAddr | Opcode::VariantFieldAddr | Opcode::IndexAddr | Opcode::PointerCast
    ) || (graph.level == GraphLevel::Layout
        && matches!(graph.op(node), Opcode::PtrOffset | Opcode::IntCast)
        && matches!(graph.value_type(node), ValueType::Rust(ty) | ValueType::LayoutValue(ty)
                if matches!(graph.oracle.semantic().shape(*ty), TypeShape::Reference { .. } | TypeShape::RawPointer { .. } | TypeShape::PlaceAddress { .. })))
}

/// Whether `address` is based on a `Frozen` parameter (RS5.2 parameter root).
///
/// The query walks back through address arithmetic, retyping, the data
/// pointer of a wide reference and value Phis whose every input qualifies.
/// A pointer loaded from memory, returned by a call or converted from an
/// integer is not based on the parameter, so `&&T` gives its inner pointer no
/// permission. rustc-O marks such parameters `noalias readonly` (D1): no
/// access during this execution writes memory reached through them.
pub fn is_frozen_parameter_address(graph: &FunctionGraph, address: NodeId) -> bool {
    if !graph.arg_permissions.iter().any(|permission| permission.is_frozen()) {
        return false;
    }
    let mut on_stack = HashSet::default();
    let mut budget = 32usize;
    frozen_parameter_based(graph, address, &mut on_stack, &mut budget)
}

fn frozen_parameter_based(
    graph: &FunctionGraph,
    node: NodeId,
    on_stack: &mut HashSet<NodeId>,
    budget: &mut usize,
) -> bool {
    let mut node = graph.resolve(node);
    loop {
        if node.is_none() || *budget == 0 {
            return false;
        }
        *budget -= 1;
        match graph.op(node) {
            Opcode::Projection if graph.resolve(graph.input(node, 0)) == graph.start => {
                let payload = graph.payload(node) as usize;
                return payload >= 2 && graph.arg_permission(payload - 2).is_frozen();
            }
            Opcode::PointerCast
            | Opcode::FieldAddr
            | Opcode::VariantFieldAddr
            | Opcode::IndexAddr
            | Opcode::WidePtr => node = graph.resolve(graph.input(node, 0)),
            // The data pointer of a wide reference shares its provenance.
            Opcode::Field
                if graph.payload(node) == 0
                    && matches!(
                        graph.value_type(graph.resolve(graph.input(node, 0))),
                        ValueType::Rust(ty) if matches!(
                            graph.oracle.semantic().shape(*ty),
                            TypeShape::Reference { .. }
                        )
                    ) =>
            {
                node = graph.resolve(graph.input(node, 0))
            }
            // A Phi on the current path stands for the values it merges: if
            // every other input is parameter-based, so is every iteration.
            Opcode::Phi if !matches!(graph.value_type(node), ValueType::Effect) => {
                if !on_stack.insert(node) {
                    return true;
                }
                let inputs: Vec<_> = graph.inputs(node)[1..].to_vec();
                let based = inputs
                    .into_iter()
                    .all(|input| frozen_parameter_based(graph, input, on_stack, budget));
                on_stack.remove(&node);
                return based;
            }
            _ => return false,
        }
    }
}
