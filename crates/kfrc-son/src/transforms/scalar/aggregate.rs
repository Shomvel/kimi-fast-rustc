//! Eliminate semantic packaging before layout introduces storage and tag bits.
//!
//! Projections observe only the selected field. In particular, a poison field
//! does not poison its siblings, and extracting an inactive variant produces
//! poison rather than observing its payload. No rule reads layout information.

use std::collections::VecDeque;

use crate::graph::{FunctionGraph, GraphLevel, NodeId, Opcode};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AggregateStats {
    pub fields_folded: usize,
    pub variants_folded: usize,
    pub aggregates_simplified: usize,
}

pub(crate) fn run(graph: &mut FunctionGraph) -> AggregateStats {
    let mut stats = AggregateStats::default();
    if !matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized) {
        return stats;
    }
    let mut worklist = Worklist::new(graph);
    while let Some(node) = worklist.pop() {
        if graph.is_dead(node) || !graph.control(node).is_none() {
            continue;
        }
        let old_len = graph.len();
        let Some(replacement) = fold(graph, node) else {
            continue;
        };
        if replacement == node || graph.value_type(node) != graph.value_type(replacement) {
            continue;
        }
        match graph.op(node) {
            Opcode::Field => stats.fields_folded += 1,
            Opcode::InsertField | Opcode::PointerCast | Opcode::WidePtr => {
                stats.aggregates_simplified += 1
            }
            _ => stats.variants_folded += 1,
        }
        graph.replace(node, replacement);
        worklist.replaced(graph, node, replacement, old_len);
    }
    stats
}

pub(super) fn fold(graph: &mut FunctionGraph, node: NodeId) -> Option<NodeId> {
    let source = graph.resolve(*graph.inputs(node).first()?);
    match graph.op(node) {
        Opcode::Field => field(graph, node, source),
        // PointerCast changes only the semantic pointee label. Retain the
        // original value (including poison and allocation identity), never an
        // integer-derived address or a computed offset.
        Opcode::PointerCast if graph.value_type(node) == graph.value_type(source) => Some(source),
        Opcode::PointerCast
            if graph.op(source) == Opcode::PointerCast && graph.control(source).is_none() =>
        {
            let original = graph.resolve(graph.input(source, 0));
            (graph.value_type(node) == graph.value_type(original)).then_some(original)
        }
        Opcode::WidePtr => reconstructed_pointer(graph, node),
        Opcode::InsertField if graph.op(source) == Opcode::Aggregate => {
            let index = graph.payload(node) as usize;
            let value = graph.resolve(*graph.inputs(node).get(1)?);
            let mut fields = graph.inputs(source).to_vec();
            *fields.get_mut(index)? = value;
            Some(graph.add_node(
                Opcode::Aggregate,
                NodeId::NONE,
                &fields,
                0,
                graph.value_type(node).clone(),
            ))
        }
        Opcode::VariantOf | Opcode::Discriminant if graph.op(source) == Opcode::MakeVariant => {
            let variant = graph.payload(source);
            let value = if graph.op(node) == Opcode::VariantOf {
                u128::from(variant)
            } else {
                let ty = graph.rust_type(source)?;
                graph.oracle.semantic().discriminant(ty, variant)?
            };
            Some(graph.add_constant(value, graph.rust_type(node)?))
        }
        Opcode::VariantField if graph.op(source) == Opcode::MakeVariant => {
            let &(variant, index) = graph.variant_fields.get(graph.payload(node) as usize)?;
            if variant == graph.payload(source) {
                Some(graph.resolve(*graph.inputs(source).get(index as usize)?))
            } else {
                Some(graph.add_node(
                    Opcode::Undefined,
                    NodeId::NONE,
                    &[],
                    0,
                    graph.value_type(node).clone(),
                ))
            }
        }
        _ => None,
    }
}

fn reconstructed_pointer(graph: &FunctionGraph, node: NodeId) -> Option<NodeId> {
    if graph.inputs(node).len() != 2 {
        return None;
    }
    let mut original = None;
    for (index, &component) in graph.inputs(node).iter().enumerate() {
        let component = graph.resolve(component);
        if graph.op(component) != Opcode::Field
            || graph.payload(component) as usize != index
            || !graph.control(component).is_none()
        {
            return None;
        }
        let source = graph.resolve(graph.input(component, 0));
        if graph.value_type(source) != graph.value_type(node)
            || original.is_some_and(|original| original != source)
        {
            return None;
        }
        original = Some(source);
    }
    original
}

fn field(graph: &mut FunctionGraph, node: NodeId, mut source: NodeId) -> Option<NodeId> {
    let original = source;
    let field = graph.payload(node);
    // A valid graph can have cycles through Phi, but not an InsertField-only
    // cycle. The bound also avoids hanging on an unverified malformed input.
    for _ in 0..graph.len() {
        match graph.op(source) {
            Opcode::Aggregate | Opcode::WidePtr => {
                return Some(graph.resolve(*graph.inputs(source).get(field as usize)?));
            }
            Opcode::Repeat => return Some(graph.resolve(graph.input(source, 0))),
            Opcode::InsertField => {
                if graph.payload(source) == field {
                    return Some(graph.resolve(*graph.inputs(source).get(1)?));
                }
                source = graph.resolve(*graph.inputs(source).first()?);
            }
            _ => {
                return (source != original).then(|| {
                    graph.add_node(
                        Opcode::Field,
                        NodeId::NONE,
                        &[source],
                        field,
                        graph.value_type(node).clone(),
                    )
                });
            }
        }
    }
    None
}

/// Forwarding can expose a constructor to users created before its fields.
/// Revisit only those users, extending the local use map for newly built nodes.
struct Worklist {
    queue: VecDeque<NodeId>,
    queued: Vec<bool>,
    users: Vec<Vec<NodeId>>,
}

impl Worklist {
    fn new(graph: &FunctionGraph) -> Self {
        let mut result = Self {
            queue: VecDeque::new(),
            queued: vec![false; graph.len()],
            users: vec![Vec::new(); graph.len()],
        };
        for node in graph.ids() {
            result.record(graph, node);
        }
        result
    }

    fn push(&mut self, node: NodeId) {
        if !self.queued[node.index()] {
            self.queued[node.index()] = true;
            self.queue.push_back(node);
        }
    }

    fn pop(&mut self) -> Option<NodeId> {
        let node = self.queue.pop_front()?;
        self.queued[node.index()] = false;
        Some(node)
    }

    fn record(&mut self, graph: &FunctionGraph, node: NodeId) {
        for &input in graph.inputs(node) {
            let input = graph.resolve(input);
            if !input.is_none() {
                self.users[input.index()].push(node);
            }
        }
        if matches!(
            graph.op(node),
            Opcode::Field
                | Opcode::PointerCast
                | Opcode::WidePtr
                | Opcode::InsertField
                | Opcode::VariantOf
                | Opcode::VariantField
                | Opcode::Discriminant
        ) {
            self.push(node);
        }
    }

    fn replaced(
        &mut self,
        graph: &FunctionGraph,
        node: NodeId,
        replacement: NodeId,
        old_len: usize,
    ) {
        self.queued.resize(graph.len(), false);
        self.users.resize_with(graph.len(), Vec::new);
        for index in old_len..graph.len() {
            self.record(graph, NodeId(index as u32));
        }
        let users = std::mem::take(&mut self.users[node.index()]);
        for user in users {
            self.users[replacement.index()].push(user);
            self.push(user);
        }
    }
}
