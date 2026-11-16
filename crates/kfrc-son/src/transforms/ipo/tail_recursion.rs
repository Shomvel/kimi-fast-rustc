//! Turn direct self calls immediately returned by the caller into backedges.
//!
//! This is the loop-forming part of LLVM's TailCallElim. Frame allocations,
//! variadics, accumulators and work after the call require separate proofs.
use crate::analysis::uses::{UseKind, Uses};
use crate::verify::verify_pipeline_step;
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType, VerifyError};
use kfrc_body::PassMode;

mod returns;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TailRecursionStats {
    pub calls_eliminated: usize,
    pub phis_created: usize,
    pub returns_duplicated: usize,
}

struct Tail {
    call: NodeId,
    ret: NodeId,
    projections: Vec<NodeId>,
}

/// Preserve every call-boundary poison observation when replacing the call
/// with a jump. The entry's real parameters remain outside the new loop.
pub fn eliminate_tail_recursion(
    graph: &mut FunctionGraph,
) -> Result<TailRecursionStats, VerifyError> {
    let mut stats = TailRecursionStats::default();
    if graph.level != GraphLevel::Specialized
        || graph.abi.c_variadic
        || graph
            .abi
            .args
            .iter()
            .any(|mode| matches!(mode, PassMode::Indirect | PassMode::IndirectOnStack))
        || graph.abi.args.len() > 32
        || graph.live_count() > 16_384
        || graph
            .ids()
            .any(|node| matches!(graph.op(node), Opcode::LocalSlot | Opcode::StackSlot))
        || !graph.ids().any(|node| {
            graph.op(node) == Opcode::Call
                && graph.callees[graph.payload(node) as usize].symbol == graph.symbol
        })
    {
        return Ok(stats);
    }
    verify_pipeline_step(graph)?;
    graph.resolve_edges();
    stats.returns_duplicated = returns::run(graph)?;
    verify_pipeline_step(graph)?;
    let uses = Uses::new(graph);
    let tails: Vec<_> = graph
        .ids()
        .filter_map(|node| candidate(graph, &uses, node))
        .take(16)
        .collect();
    if tails.is_empty() {
        return Ok(stats);
    }
    let ValueType::Tuple(parameters) = graph.value_type(graph.start).clone() else {
        unreachable!("verified Start tuple")
    };
    let original: Vec<_> = graph.ids().collect();
    let entry = projection(graph, graph.start, 0, ValueType::Control);
    let effect = projection(graph, graph.start, 1, ValueType::Effect);
    let incoming: Vec<_> = parameters[2..]
        .iter()
        .enumerate()
        .map(|(index, ty)| projection(graph, graph.start, index as u32 + 2, ty.clone()))
        .collect();
    let entry_jump = graph.add_node(Opcode::Jump, entry, &[], 0, ValueType::Control);
    let header = graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry_jump],
        0,
        ValueType::Control,
    );
    let effect_phi = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, effect],
        0,
        ValueType::Effect,
    );
    let phis: Vec<_> = incoming
        .iter()
        .zip(&parameters[2..])
        .map(|(&arg, ty)| graph.add_node(Opcode::Phi, NodeId::NONE, &[header, arg], 0, ty.clone()))
        .collect();
    // A Start may have several equivalent projections. Rewrite their users,
    // keeping the original producers alive for the initial Phi operands.
    let mut mapping = vec![NodeId::NONE; graph.len()];
    for &node in &original {
        if graph.op(node) == Opcode::Projection && graph.input(node, 0) == graph.start {
            mapping[node.index()] = match graph.payload(node) {
                0 => header,
                1 => effect_phi,
                slot => phis[slot as usize - 2],
            };
        }
    }
    for &node in &original {
        if node == graph.start || !mapping[node.index()].is_none() {
            continue;
        }
        for index in 0..graph.inputs(node).len() {
            let input = graph.input(node, index);
            if !mapping[input.index()].is_none() {
                graph.set_input(node, index, mapping[input.index()]);
            }
        }
        let control = graph.control(node);
        if !control.is_none() && !mapping[control.index()].is_none() {
            graph.set_control(node, mapping[control.index()]);
        }
    }
    let mut predecessors = vec![entry_jump];
    let mut effects = vec![header, effect];
    let mut arguments: Vec<_> = incoming.iter().map(|&arg| vec![header, arg]).collect();
    for tail in tails {
        let args = graph.inputs(tail.call)[1..].to_vec();
        let mut control = graph.control(tail.call);
        let mut effect = graph.input(tail.call, 0);
        for &arg in &args {
            (control, effect) = super::splice::require_defined(graph, control, effect, arg);
        }
        predecessors.push(graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
        effects.push(effect);
        for (values, arg) in arguments.iter_mut().zip(args) {
            values.push(arg);
        }
        graph.remove_node(tail.ret);
        for projection in tail.projections {
            graph.remove_node(projection);
        }
        graph.remove_node(tail.call);
        stats.calls_eliminated += 1;
    }
    graph.set_inputs(header, &predecessors);
    graph.set_inputs(effect_phi, &effects);
    for (phi, values) in phis.into_iter().zip(arguments) {
        graph.set_inputs(phi, &values);
    }
    stats.phis_created = parameters.len() - 1;
    graph.resolve_edges();
    verify_pipeline_step(graph)?;
    Ok(stats)
}

fn projection(graph: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph
        .find_projection(source, slot)
        .unwrap_or_else(|| graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty))
}

fn candidate(graph: &FunctionGraph, uses: &Uses, call: NodeId) -> Option<Tail> {
    if graph.op(call) != Opcode::Call {
        return None;
    }
    let target = &graph.callees[graph.payload(call) as usize];
    if target.symbol != graph.symbol || target.abi != graph.abi {
        return None;
    }
    let mut ret = None;
    let mut projections = Vec::new();
    for usage in uses.of(call) {
        let projection = usage.user;
        if usage.kind() != UseKind::Operand(0) || graph.op(projection) != Opcode::Projection {
            return None;
        }
        projections.push(projection);
        for usage in uses.of(projection) {
            let expected = match graph.payload(projection) {
                0 => UseKind::Control,
                1 => UseKind::Operand(0),
                2 => UseKind::Operand(1),
                _ => return None,
            };
            if usage.kind() != expected
                || graph.op(usage.user) != Opcode::Return
                || ret.is_some_and(|ret| ret != usage.user)
            {
                return None;
            }
            ret = Some(usage.user);
        }
    }
    let ret = ret?;
    let is_result = |node, slot| {
        graph.op(node) == Opcode::Projection
            && graph.input(node, 0) == call
            && graph.payload(node) == slot
    };
    if !is_result(graph.control(ret), 0)
        || !is_result(graph.input(ret, 0), 1)
        || !is_result(graph.input(ret, 1), 2)
    {
        return None;
    }
    Some(Tail {
        call,
        ret,
        projections,
    })
}
