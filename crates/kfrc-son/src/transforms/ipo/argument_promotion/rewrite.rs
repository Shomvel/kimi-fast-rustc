use super::plan::Plan;
use crate::{ArgPermission, FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::RecordedTypes;

struct Imported {
    oracle: RecordedTypes,
    paths: Vec<Vec<Vec<(u32, ValueType)>>>,
    values: Vec<Vec<ValueType>>,
}

fn import(caller: &FunctionGraph, source: &FunctionGraph, plan: &Plan) -> Option<Imported> {
    let mut oracle = caller.oracle.clone();
    let mut types = super::super::types::TypeImport::new(&source.oracle, &mut oracle);
    let start = types.value_type(source.value_type(source.start)).ok()?;
    let ValueType::Tuple(parameters) = start else {
        return None;
    };
    let calls: Vec<_> = caller
        .ids()
        .filter(|&node| {
            caller.op(node) == Opcode::Call
                && caller.callees[caller.payload(node) as usize].symbol == source.symbol
        })
        .collect();
    for call in calls {
        let target = &caller.callees[caller.payload(call) as usize];
        if target.abi != source.abi
            || caller.inputs(call).len() + 1 != parameters.len()
            || caller.inputs(call)[1..]
                .iter()
                .zip(&parameters[2..])
                .any(|(&arg, ty)| caller.value_type(arg) != ty)
        {
            return None;
        }
    }
    // Even unused interning entries must agree: call permission metadata is
    // keyed by symbol and the verifier checks all targets of that symbol.
    if caller
        .callees
        .iter()
        .any(|target| target.symbol == source.symbol && target.abi != source.abi)
    {
        return None;
    }
    let mut paths = Vec::new();
    let mut values = Vec::new();
    for argument in &plan.arguments {
        let mut arg_paths = Vec::new();
        let mut arg_values = Vec::new();
        for part in &argument.parts {
            arg_paths.push(
                part.path
                    .iter()
                    .map(|(field, ty)| Some((*field, types.value_type(ty).ok()?)))
                    .collect::<Option<Vec<_>>>()?,
            );
            arg_values.push(types.value_type(&ValueType::Rust(part.ty)).ok()?);
        }
        paths.push(arg_paths);
        values.push(arg_values);
    }
    types.finish().ok()?;
    Some(Imported {
        oracle,
        paths,
        values,
    })
}

pub(super) fn calls(
    caller: &mut FunctionGraph,
    source: &FunctionGraph,
    plan: &Plan,
) -> Option<usize> {
    let imported = import(caller, source, plan)?;
    let calls: Vec<_> = caller
        .ids()
        .filter(|&node| {
            caller.op(node) == Opcode::Call
                && caller.callees[caller.payload(node) as usize].symbol == source.symbol
        })
        .collect();
    caller.oracle = imported.oracle;
    for &call in &calls {
        let args = caller.inputs(call)[1..].to_vec();
        let mut control = caller.control(call);
        let mut effect = caller.input(call, 0);
        // The old boundary observes every argument before the callee reads.
        // Keep these observations even for the pointer removed from its ABI.
        for &arg in &args {
            (control, effect) = super::super::splice::require_defined(caller, control, effect, arg);
        }
        let mut parameters = Vec::new();
        for (index, &arg) in args.iter().enumerate() {
            let Some((arg_index, argument)) = plan
                .arguments
                .iter()
                .enumerate()
                .find(|(_, argument)| argument.index == index)
            else {
                parameters.push(arg);
                continue;
            };
            for (part_index, _) in argument.parts.iter().enumerate() {
                let mut address = arg;
                for (field, ty) in &imported.paths[arg_index][part_index] {
                    address = caller.add_node(
                        Opcode::FieldAddr,
                        NodeId::NONE,
                        &[address],
                        *field,
                        ty.clone(),
                    );
                }
                let ty = imported.values[arg_index][part_index].clone();
                let load = caller.add_node(
                    Opcode::LoadValue,
                    control,
                    &[effect, address],
                    0,
                    ValueType::Tuple(vec![ValueType::Control, ValueType::Effect, ty.clone()]),
                );
                control = projection(caller, load, 0, ValueType::Control);
                effect = projection(caller, load, 1, ValueType::Effect);
                parameters.push(projection(caller, load, 2, ty));
            }
        }
        let mut inputs = vec![effect];
        inputs.extend(parameters);
        caller.set_inputs(call, &inputs);
        caller.set_control(call, control);
    }
    for target in &mut caller.callees {
        if target.symbol == source.symbol {
            target.abi = plan.abi.clone();
        }
    }
    if let Some(permissions) = caller.callee_permissions.get(&source.symbol).cloned() {
        caller
            .callee_permissions
            .insert(source.symbol.clone(), remap_permissions(&permissions, plan));
    }
    caller.callee_attrs.remove(&source.symbol);
    Some(calls.len())
}

pub(super) fn definition(graph: &mut FunctionGraph, plan: &Plan) {
    let original: Vec<_> = graph.ids().collect();
    let projections: Vec<Vec<_>> = (0..graph.abi.args.len())
        .map(|index| {
            original
                .iter()
                .copied()
                .filter(|&node| {
                    graph.op(node) == Opcode::Projection
                        && graph.input(node, 0) == graph.start
                        && graph.payload(node) as usize == index + 2
                })
                .collect()
        })
        .collect();
    let mut next_index = 0u32;
    for index in 0..graph.abi.args.len() {
        if let Some(argument) = plan
            .arguments
            .iter()
            .find(|argument| argument.index == index)
        {
            for part in &argument.parts {
                let value =
                    projection(graph, graph.start, next_index + 2, ValueType::Rust(part.ty));
                next_index += 1;
                for &load in &part.loads {
                    let control = graph.control(load);
                    let effect = graph.input(load, 0);
                    for &node in &original {
                        if graph.op(node) == Opcode::Projection && graph.input(node, 0) == load {
                            let replacement = match graph.payload(node) {
                                0 => control,
                                1 => effect,
                                2 => value,
                                _ => unreachable!("verified load projection"),
                            };
                            graph.replace(node, replacement);
                        }
                    }
                    graph.remove_node(load);
                }
            }
            for &address in &argument.addresses {
                graph.remove_node(address);
            }
            graph.remove_node(argument.root);
        } else {
            for &node in &projections[index] {
                graph.set_payload(node, next_index + 2);
            }
            next_index += 1;
        }
    }
    if !graph.arg_permissions.is_empty() {
        graph.arg_permissions = remap_permissions(&graph.arg_permissions, plan);
    }
    graph.abi = plan.abi.clone();
    graph.set_value_type(graph.start, ValueType::Tuple(plan.start.clone()));
}

fn remap_permissions(old: &[ArgPermission], plan: &Plan) -> Vec<ArgPermission> {
    let mut result = Vec::new();
    for (index, &permission) in old.iter().enumerate() {
        if let Some(argument) = plan
            .arguments
            .iter()
            .find(|argument| argument.index == index)
        {
            result.extend(std::iter::repeat_n(
                ArgPermission::None,
                argument.parts.len(),
            ));
        } else {
            result.push(permission);
        }
    }
    result
}

fn projection(graph: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}
