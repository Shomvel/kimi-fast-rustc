use super::*;
use crate::analysis::cfg::is_control_flow;

pub(super) fn uses(graph: &FunctionGraph, fuel: &mut Fuel) -> Option<Uses> {
    // Pay for the dense index and both edge passes in Uses::new. Graph storage
    // itself is charged, including retired records, before allocating an index.
    fuel.take(graph.len())?;
    for node in graph.ids() {
        fuel.take(2 * (graph.inputs(node).len() + 2))?;
    }
    Some(Uses::new(graph))
}

pub(super) fn only(uses: &Uses, node: NodeId, user: NodeId, kind: UseKind) -> bool {
    matches!(uses.of(node), [u] if u.user == user && u.kind == kind)
}

pub(super) fn projection(
    graph: &FunctionGraph,
    uses: &Uses,
    node: NodeId,
    slot: u32,
    fuel: &mut Fuel,
) -> Option<NodeId> {
    let mut found = None;
    for usage in uses.of(node) {
        fuel.take(1)?;
        if usage.kind != UseKind::Operand(0)
            || graph.op(usage.user) != Opcode::Projection
            || graph.payload(usage.user) != slot
        {
            continue;
        }
        if found.replace(usage.user).is_some() {
            return None;
        }
    }
    found
}

pub(super) fn forward_join(
    graph: &FunctionGraph,
    uses: &Uses,
    region: NodeId,
    fuel: &mut Fuel,
) -> Option<()> {
    let edges = graph.inputs(region);
    if graph.op(region) != Opcode::Region || edges.len() != 2 || edges[0] == edges[1] {
        return None;
    }
    // A header dominates its backedge. Both predecessors must instead be
    // reachable from Start while avoiding this region. This admits a forward
    // diamond inside an outer loop without accepting a loop-carried Phi.
    fuel.take(graph.len())?;
    let mut seen = vec![false; graph.len()];
    let mut queue = vec![graph.start];
    while let Some(node) = queue.pop() {
        fuel.take(1)?;
        if node == region || seen[node.index()] {
            continue;
        }
        seen[node.index()] = true;
        if edges.iter().all(|edge| seen[edge.index()]) {
            return Some(());
        }
        for usage in uses.of(node) {
            fuel.take(1)?;
            let user = usage.user;
            if !is_control_flow(graph, user) {
                continue;
            }
            let edge = usage.kind == UseKind::Control
                || graph.op(user) == Opcode::Region
                || (graph.op(user) == Opcode::Projection
                    && usage.kind == UseKind::Operand(0)
                    && graph.value_type(user) == &ValueType::Control);
            if edge && !seen[user.index()] {
                queue.push(user);
            }
        }
    }
    None
}
