use super::{ConditionalReturnStats, Selector, Summary};
use crate::transforms::ipo::splice;
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{TypeOracle, TypeShape};
use std::collections::BTreeSet;

pub(super) fn literal_selectors(
    graph: &FunctionGraph,
    call: NodeId,
    summary: &Summary,
) -> [Option<u128>; 2] {
    [&summary.lhs, &summary.rhs].map(|selector| {
        let &argument = graph.inputs(call).get(selector.parameter + 1)?;
        literal(graph, argument, &selector.fields, &mut 32)
    })
}

/// Only ordinary scalar literals and structural constructors supply this
/// profitability fact. The shared representation certificate may identify
/// one executed conversion's source metadata; that fixed conversion remains.
/// Loads, Phis and opaque results cannot supply a new literal fact.
fn literal(graph: &FunctionGraph, node: NodeId, fields: &[u32], fuel: &mut usize) -> Option<u128> {
    *fuel = fuel.checked_sub(1)?;
    let node = graph.resolve(node);
    if !graph.control(node).is_none() {
        return None;
    }
    match graph.op(node) {
        Opcode::Const if fields.is_empty() => graph
            .rust_type(node)
            .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Int { .. }))
            .then(|| graph.constant_value(node)),
        Opcode::Field => {
            let mut path = Vec::with_capacity(fields.len() + 1);
            path.push(graph.payload(node));
            path.extend_from_slice(fields);
            literal(graph, graph.input(node, 0), &path, fuel)
        }
        Opcode::Aggregate | Opcode::WidePtr if !fields.is_empty() => literal(
            graph,
            *graph.inputs(node).get(fields[0] as usize)?,
            &fields[1..],
            fuel,
        ),
        Opcode::Projection if fields == [1] => {
            let (source, metadata) =
                crate::transforms::scalar::aggregate::wideptr_metadata_source(graph, node)?;
            if !matches!(graph.oracle.shape(metadata), TypeShape::Int { .. }) {
                return None;
            }
            literal(graph, source, fields, fuel)
        }
        _ => None,
    }
}

fn select(
    graph: &mut FunctionGraph,
    args: &[NodeId],
    selector: &Selector,
    literal: Option<u128>,
) -> NodeId {
    let mut value = args[selector.parameter];
    if let Some(bits) = literal {
        let mut ty = graph.rust_type(value).expect("verified selector root");
        for &field in &selector.fields {
            ty = graph
                .oracle
                .semantic()
                .field_type(ty, None, field)
                .expect("imported selector field");
        }
        // The fact is conditional on all original arguments being defined and
        // their fixed conversions executing; those observations still precede
        // this branch. No representation node or parameter is replaced.
        return graph.add_constant(bits, ty);
    }
    for &field in &selector.fields {
        let ty = graph
            .oracle
            .semantic()
            .field_type(
                graph.rust_type(value).expect("verified selector root"),
                None,
                field,
            )
            .expect("imported selector field");
        value = graph.add_node(
            Opcode::Field,
            NodeId::NONE,
            &[value],
            field,
            ValueType::Rust(ty),
        );
    }
    value
}

pub(super) fn split(
    graph: &mut FunctionGraph,
    call: NodeId,
    summary: &Summary,
    literals: [Option<u128>; 2],
    stats: &mut ConditionalReturnStats,
) {
    let args: Vec<_> = graph.inputs(call)[1..]
        .iter()
        .map(|&node| graph.resolve(node))
        .collect();
    let old_projections: BTreeSet<_> = graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::Projection && graph.input(node, 0) == call)
        .collect();
    stats.dirty_nodes.extend(graph.ids().filter(|&node| {
        graph
            .inputs(node)
            .iter()
            .any(|input| old_projections.contains(input))
            || old_projections.contains(&graph.control(node))
    }));
    stats
        .argument_slots
        .extend(splice::argument_slots(graph, &args));

    let ValueType::Tuple(outputs) = graph.value_type(call).clone() else {
        unreachable!("verified call boundary")
    };
    let mut control = graph.resolve(graph.control(call));
    let mut effect = graph.resolve(graph.input(call, 0));
    for &argument in &args {
        (control, effect) = splice::require_defined(graph, control, effect, argument);
    }
    // Select fields of the already-produced arguments. This retains fixed
    // producers and observes poison in data pointers and unused siblings too.
    let lhs = select(graph, &args, &summary.lhs, literals[0]);
    let rhs = select(graph, &args, &summary.rhs, literals[1]);
    let condition = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[lhs, rhs],
        summary.predicate as u32,
        outputs[2].clone(),
    );
    let branch = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let bypass = projection(graph, branch, 0, ValueType::Control);
    let fallback = projection(graph, branch, 1, ValueType::Control);
    graph.set_control(call, fallback);
    graph.set_input(call, 0, effect);

    // Fresh raw outputs prevent replace(old_projection, merged_phi) from also
    // replacing the merge's own fallback inputs and creating a value cycle.
    let raw_control = projection(graph, call, 0, ValueType::Control);
    let raw_effect = projection(graph, call, 1, ValueType::Effect);
    let raw_value = projection(graph, call, 2, outputs[2].clone());
    let bypass_jump = graph.add_node(Opcode::Jump, bypass, &[], 0, ValueType::Control);
    let fallback_jump = graph.add_node(Opcode::Jump, raw_control, &[], 0, ValueType::Control);
    let region = graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[bypass_jump, fallback_jump],
        0,
        ValueType::Control,
    );
    let ValueType::Rust(boolean) = &outputs[2] else {
        unreachable!("constant bool result")
    };
    let boolean = *boolean;
    let value = graph.add_constant(u128::from(summary.result), boolean);
    let merged_value = graph.add_node(
        Opcode::Phi,
        region,
        &[region, value, raw_value],
        0,
        ValueType::Rust(boolean),
    );
    let merged_effect = graph.add_node(
        Opcode::Phi,
        region,
        &[region, effect, raw_effect],
        0,
        ValueType::Effect,
    );
    for projection in old_projections {
        let replacement = match graph.payload(projection) {
            0 => region,
            1 => merged_effect,
            2 => merged_value,
            _ => unreachable!("verified call projection"),
        };
        graph.replace(projection, replacement);
    }
    stats
        .dirty_nodes
        .extend([region, merged_value, merged_effect, condition]);
    stats.calls_split += 1;
}

fn projection(graph: &mut FunctionGraph, source: NodeId, index: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty)
}
