//! Summarize exact argument returns without cloning a callee or spending the
//! general inlining budget. Logical call observations remain in source order.
use super::{GraphProvider, InlineError, InlineSkip, splice, types::TypeImport};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType, verify_graph};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default)]
pub struct IdentityFoldStats {
    pub calls_examined: usize,
    pub calls_folded: usize,
    pub summary_nodes_visited: usize,
    pub skipped: Vec<(String, InlineSkip)>,
    /// Stable until compaction; consumers of erased call projections.
    pub dirty_nodes: Vec<NodeId>,
    pub argument_slots: Vec<NodeId>,
}

/// `seeds` limits consideration to these call nodes; None considers every call.
/// The whole edit is transactional. Source verification failures are errors;
/// unsupported summaries or incompatible call boundaries are ordinary skips.
pub fn fold_identity_calls(
    graph: &mut FunctionGraph,
    provider: &(impl GraphProvider + ?Sized),
    seeds: Option<&[NodeId]>,
) -> Result<IdentityFoldStats, InlineError> {
    if graph.level != GraphLevel::Specialized {
        return Err(InlineError::NotSpecialized);
    }
    verify_graph(graph).map_err(InlineError::Verification)?;
    let mut calls = seeds.map_or_else(
        || graph.ids().collect::<Vec<_>>(),
        |seeds| seeds.iter().map(|&n| graph.resolve(n)).collect(),
    );
    calls.retain(|&n| !n.is_none() && !graph.is_dead(n) && graph.op(n) == Opcode::Call);
    calls.sort_unstable();
    calls.dedup();
    let mut stats = IdentityFoldStats::default();
    let mut summaries = HashMap::new();
    let mut transaction: Option<FunctionGraph> = None;
    for call in calls {
        let current = transaction.as_ref().unwrap_or(graph);
        let target = current.callees[current.payload(call) as usize].clone();
        stats.calls_examined += 1;
        let Some(source) = provider.graph(&target.symbol) else {
            stats.skipped.push((target.symbol, InlineSkip::Missing));
            continue;
        };
        let summary = if let Some(summary) = summaries.get(&target.symbol) {
            *summary
        } else {
            stats.summary_nodes_visited += source.ids().count();
            let summary = summarize(source);
            if summary.is_some() {
                crate::verify::check(source).map_err(InlineError::Verification)?;
            }
            summaries.insert(target.symbol.clone(), summary);
            summary
        };
        let Some(argument) = summary else {
            stats.skipped.push((target.symbol, InlineSkip::Unsupported));
            continue;
        };
        if source.abi != target.abi {
            stats.skipped.push((target.symbol, InlineSkip::AbiMismatch));
            continue;
        }
        let oracle = match boundary(current, call, source, argument) {
            Ok(oracle) => oracle,
            Err(reason) => {
                stats.skipped.push((target.symbol, reason));
                continue;
            }
        };
        let candidate = transaction.get_or_insert_with(|| graph.clone());
        candidate.oracle = oracle;
        rewrite(candidate, call, argument, &mut stats);
    }
    if let Some(mut candidate) = transaction {
        candidate.resolve_edges();
        verify_graph(&mut candidate).map_err(InlineError::Verification)?;
        for nodes in [&mut stats.dirty_nodes, &mut stats.argument_slots] {
            for node in nodes.iter_mut() {
                *node = candidate.resolve(*node);
            }
            nodes.retain(|&n| !n.is_none() && !candidate.is_dead(n));
            nodes.sort_unstable();
            nodes.dedup();
        }
        *graph = candidate;
    }
    Ok(stats)
}

/// Permit only a single acyclic path, with optional Jump/Region/effect-Phi
/// forwarding shells. Every live node must be accounted for by that path.
fn summarize(graph: &FunctionGraph) -> Option<usize> {
    if graph.level != GraphLevel::Specialized
        || graph.abi.c_variadic
        || !graph.data_objects.is_empty()
    {
        return None;
    }
    let mut ret = None;
    for node in graph.ids() {
        match graph.op(node) {
            Opcode::Start if node == graph.start => {}
            Opcode::Projection if graph.input(node, 0) == graph.start => {}
            Opcode::Return if ret.is_none() => ret = Some(node),
            Opcode::Jump | Opcode::Region => {}
            Opcode::Phi if graph.value_type(node) == &ValueType::Effect => {}
            _ => return None,
        }
    }
    let ret = ret?;
    if graph.inputs(ret).len() != 2 {
        return None;
    }
    let value = graph.input(ret, 1);
    if graph.op(value) != Opcode::Projection || graph.input(value, 0) != graph.start {
        return None;
    }
    let argument = graph.payload(value).checked_sub(2)? as usize;
    let mut path = HashSet::new();
    let mut control = graph.control(ret);
    loop {
        if !path.insert(control) {
            return None;
        }
        match graph.op(control) {
            Opcode::Projection
                if graph.input(control, 0) == graph.start && graph.payload(control) == 0 =>
            {
                break;
            }
            Opcode::Jump if graph.inputs(control).is_empty() => control = graph.control(control),
            Opcode::Region if graph.inputs(control).len() == 1 => {
                let incoming = graph.input(control, 0);
                if graph.op(incoming) != Opcode::Jump {
                    return None;
                }
                control = incoming;
            }
            _ => return None,
        }
    }
    let mut effects = HashSet::new();
    let mut effect = graph.input(ret, 0);
    loop {
        if !effects.insert(effect) {
            return None;
        }
        match graph.op(effect) {
            Opcode::Projection
                if graph.input(effect, 0) == graph.start && graph.payload(effect) == 1 =>
            {
                break;
            }
            Opcode::Phi
                if graph.inputs(effect).len() == 2 && path.contains(&graph.input(effect, 0)) =>
            {
                effect = graph.input(effect, 1);
            }
            _ => return None,
        }
    }
    graph
        .ids()
        .all(|n| match graph.op(n) {
            Opcode::Jump | Opcode::Region => path.contains(&n),
            Opcode::Phi => effects.contains(&n),
            _ => true,
        })
        .then_some(argument)
}

fn boundary(
    graph: &FunctionGraph,
    call: NodeId,
    source: &FunctionGraph,
    argument: usize,
) -> Result<kfrc_body::RecordedTypes, InlineSkip> {
    let mut oracle = graph.oracle.clone();
    let mut import = TypeImport::new(&source.oracle, &mut oracle);
    let start = import.value_type(source.value_type(source.start))?;
    import.finish()?;
    let ValueType::Tuple(parameters) = start else {
        return Err(InlineSkip::TypeConflict);
    };
    let args = &graph.inputs(call)[1..];
    let ValueType::Tuple(results) = graph.value_type(call) else {
        return Err(InlineSkip::TypeConflict);
    };
    if args.len() + 2 != parameters.len()
        || argument >= args.len()
        || results.len() != 3
        || args
            .iter()
            .zip(&parameters[2..])
            .any(|(&arg, ty)| graph.value_type(graph.resolve(arg)) != ty)
        || results[2] != parameters[argument + 2]
    {
        return Err(InlineSkip::TypeConflict);
    }
    Ok(oracle)
}

fn rewrite(
    graph: &mut FunctionGraph,
    call: NodeId,
    argument: usize,
    stats: &mut IdentityFoldStats,
) {
    let args: Vec<_> = graph.inputs(call)[1..]
        .iter()
        .map(|&n| graph.resolve(n))
        .collect();
    let projections: HashSet<_> = graph
        .ids()
        .filter(|&n| graph.op(n) == Opcode::Projection && graph.input(n, 0) == call)
        .collect();
    stats.dirty_nodes.extend(graph.ids().filter(|&n| {
        graph.inputs(n).iter().any(|n| projections.contains(n))
            || projections.contains(&graph.control(n))
    }));
    stats
        .argument_slots
        .extend(splice::argument_slots(graph, &args));
    let mut control = graph.resolve(graph.control(call));
    let mut effect = graph.resolve(graph.input(call, 0));
    for &arg in &args {
        (control, effect) = splice::require_defined(graph, control, effect, arg);
    }
    let value = args[argument];
    (control, effect) = splice::require_defined(graph, control, effect, value);
    for projection in projections {
        let replacement = match graph.payload(projection) {
            0 => control,
            1 => effect,
            2 => value,
            _ => unreachable!("verified call projection"),
        };
        graph.replace(projection, replacement);
    }
    graph.remove_node(call);
    stats.calls_folded += 1;
}
