//! Duplicate a shared return onto a self-call edge before forming a loop.
//! Region predecessor slots and every value/effect Phi change together.
use crate::analysis::uses::{UseKind, Uses};
use crate::transforms::utils::cfg::CfgEditor;
use crate::{FunctionGraph, NodeId, Opcode, ValueType, VerifyError};
use std::collections::BTreeMap;

struct Split {
    region: NodeId,
    slot: usize,
    jump: NodeId,
    control: NodeId,
    effect: NodeId,
    value: NodeId,
}

pub(super) fn run(graph: &mut FunctionGraph) -> Result<usize, VerifyError> {
    let uses = Uses::new(graph);
    let plans: Vec<_> = graph
        .ids()
        .filter_map(|call| plan(graph, &uses, call))
        .take(16)
        .collect();
    let mut masks: BTreeMap<NodeId, Vec<bool>> = BTreeMap::new();
    for plan in &plans {
        masks
            .entry(plan.region)
            .or_insert_with(|| vec![true; graph.inputs(plan.region).len()])[plan.slot] = false;
    }
    // Leave a non-tail predecessor for the original return. An all-recursive
    // exit requires retiring that whole unreachable shell, outside this subset.
    masks.retain(|_, keep| keep.iter().any(|&keep| keep));
    let mut count = 0;
    for plan in plans {
        if !masks.contains_key(&plan.region) {
            continue;
        }
        graph.add_node(
            Opcode::Return,
            plan.control,
            &[plan.effect, plan.value],
            0,
            ValueType::Control,
        );
        graph.remove_node(plan.jump);
        count += 1;
    }
    for (region, keep) in masks {
        CfgEditor::new(graph)
            .retain_predecessors(region, &keep)
            .map_err(|error| VerifyError::TypeMismatch {
                node: region,
                detail: error.to_string(),
            })?;
    }
    graph.resolve_edges();
    Ok(count)
}

fn plan(graph: &FunctionGraph, uses: &Uses, call: NodeId) -> Option<Split> {
    if graph.op(call) != Opcode::Call {
        return None;
    }
    let target = &graph.callees[graph.payload(call) as usize];
    if target.symbol != graph.symbol || target.abi != graph.abi {
        return None;
    }
    let control = graph.find_projection(call, 0)?;
    let effect = graph.find_projection(call, 1)?;
    let value = graph.find_projection(call, 2)?;
    let [edge] = uses.of(control) else {
        return None;
    };
    let jump = edge.user;
    if edge.kind != UseKind::Control || graph.op(jump) != Opcode::Jump {
        return None;
    }
    let [edge] = uses.of(jump) else { return None };
    let UseKind::Operand(slot) = edge.kind else {
        return None;
    };
    let region = edge.user;
    if graph.op(region) != Opcode::Region || graph.inputs(region).len() < 2 {
        return None;
    }
    let ret = uses.of(region).iter().find_map(|usage| {
        (usage.kind == UseKind::Control && graph.op(usage.user) == Opcode::Return)
            .then_some(usage.user)
    })?;
    for usage in uses.of(region) {
        if !(usage.kind == UseKind::Control && usage.user == ret
            || usage.kind == UseKind::Operand(0) && graph.op(usage.user) == Opcode::Phi)
        {
            return None;
        }
    }
    let incoming = |node| {
        if graph.op(node) == Opcode::Phi && graph.input(node, 0) == region {
            graph.inputs(node).get(slot + 1).copied()
        } else {
            Some(node)
        }
    };
    if incoming(graph.input(ret, 0)) != Some(effect) || incoming(graph.input(ret, 1)) != Some(value)
    {
        return None;
    }
    for usage in uses.of(call) {
        let projection = usage.user;
        if usage.kind != UseKind::Operand(0) || graph.op(projection) != Opcode::Projection {
            return None;
        }
        for usage in uses.of(projection) {
            match graph.payload(projection) {
                0 if usage.kind == UseKind::Control && usage.user == jump => {}
                index @ (1 | 2)
                    if usage.kind == UseKind::Operand(slot + 1)
                        && usage.user == graph.input(ret, index as usize - 1)
                        && graph.op(usage.user) == Opcode::Phi
                        && graph.input(usage.user, 0) == region => {}
                _ => return None,
            }
        }
    }
    Some(Split {
        region,
        slot,
        jump,
        control,
        effect,
        value,
    })
}
