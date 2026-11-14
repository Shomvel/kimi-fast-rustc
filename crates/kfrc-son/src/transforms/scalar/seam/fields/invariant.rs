//! Resolve one demanded field through aggregate Phi cycles. Every dependency
//! must reach the same entry value; a closed, ungrounded subcycle is unknown.

use kfrc_body::fx_hash::{HashMap, HashSet};

use crate::{FunctionGraph, NodeId, Opcode, ValueType};

pub(super) fn find(
    graph: &FunctionGraph,
    source: NodeId,
    field: u32,
    ty: &ValueType,
) -> Option<NodeId> {
    if !matches!(graph.op(source), Opcode::Phi | Opcode::InsertField) {
        return None;
    }
    let mut pending = vec![source];
    let mut seen = HashSet::default();
    let mut parents: HashMap<NodeId, Vec<NodeId>> = HashMap::default();
    let mut anchored = HashSet::default();
    let mut anchor = None;
    while let Some(node) = pending.pop() {
        let node = graph.resolve(node);
        if !seen.insert(node) {
            continue;
        }
        if !graph.control(node).is_none() {
            return None;
        }
        let terminal = match graph.op(node) {
            Opcode::Aggregate => Some(*graph.inputs(node).get(field as usize)?),
            Opcode::Repeat => Some(graph.input(node, 0)),
            Opcode::InsertField if graph.payload(node) == field => Some(graph.input(node, 1)),
            Opcode::InsertField => {
                let base = graph.resolve(graph.input(node, 0));
                parents.entry(base).or_default().push(node);
                pending.push(base);
                None
            }
            Opcode::Phi => {
                let inputs = graph.inputs(node).get(1..)?;
                if inputs.is_empty() {
                    return None;
                }
                for &input in inputs {
                    let input = graph.resolve(input);
                    parents.entry(input).or_default().push(node);
                    pending.push(input);
                }
                None
            }
            _ => return None,
        };
        if let Some(terminal) = terminal {
            let terminal = graph.resolve(terminal);
            if graph.value_type(terminal) != ty
                || !entry_value(graph, terminal)
                || anchor.is_some_and(|anchor| anchor != terminal)
            {
                return None;
            }
            anchor = Some(terminal);
            anchored.insert(node);
        }
    }
    let anchor = anchor?;
    // It is insufficient for just one arm to be grounded: another arm could
    // enter a self-contained Phi cycle. Require an anchor path for every state.
    let mut pending: Vec<_> = anchored.iter().copied().collect();
    while let Some(node) = pending.pop() {
        for &parent in parents.get(&node).into_iter().flatten() {
            if anchored.insert(parent) {
                pending.push(parent);
            }
        }
    }
    (anchored.len() == seen.len()).then_some(anchor)
}

fn entry_value(graph: &FunctionGraph, mut node: NodeId) -> bool {
    let original = node;
    let mut seen = HashSet::default();
    while seen.insert(node) {
        if !graph.control(node).is_none() {
            return false;
        }
        match graph.op(node) {
            Opcode::Const => return node == original,
            Opcode::Projection => {
                return graph.payload(node) >= 2
                    && graph.resolve(graph.input(node, 0)) == graph.start;
            }
            Opcode::Field => node = graph.resolve(graph.input(node, 0)),
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
#[path = "invariant_tests.rs"]
mod tests;
