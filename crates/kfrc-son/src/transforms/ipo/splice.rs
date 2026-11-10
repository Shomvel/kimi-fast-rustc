//! Clone a preflighted callee, preserving observations and local lifetimes.

use super::plan::Plan;
use crate::{FunctionGraph, MemoryPartition, NodeId, Opcode, ValueType};
use kfrc_body::fx_hash::HashSet;

pub(super) struct Spliced {
    pub(super) nested: Vec<NodeId>,
    pub(super) dirty: Vec<NodeId>,
    pub(super) argument_slots: Vec<NodeId>,
    pub(super) cloned_nodes: usize,
}
pub(super) fn splice(
    graph: &mut FunctionGraph,
    call: NodeId,
    source: &FunctionGraph,
    plan: Plan,
) -> Spliced {
    let Plan {
        ids,
        imported,
        oracle,
        expected_result,
        locals,
        data,
    } = plan;
    if let Some(oracle) = oracle {
        graph.oracle = oracle;
    }
    for index in data {
        let data = &source.data_objects[index];
        if !graph.data_objects.iter().any(|d| d.symbol == data.symbol) {
            graph.data_objects.push(data.clone());
        }
    }
    let arguments: Vec<_> = graph.inputs(call)[1..]
        .iter()
        .map(|n| graph.resolve(*n))
        .collect();
    let argument_slots = argument_slots(graph, &arguments);
    let projections: Vec<_> = graph
        .ids()
        .filter(|&n| graph.op(n) == Opcode::Projection && graph.input(n, 0) == call)
        .collect();
    let mut dirty: Vec<_> = graph
        .ids()
        .filter(|&n| graph.inputs(n).iter().any(|v| projections.contains(v)))
        .collect();
    let symbols: Vec<_> = source
        .symbols
        .iter()
        .map(|symbol| {
            if let Some(index) = graph.symbols.iter().position(|old| old == symbol) {
                index as u32
            } else {
                let index = graph.symbols.len() as u32;
                graph.symbols.push(symbol.clone());
                index
            }
        })
        .collect();
    let callees: Vec<_> = source
        .callees
        .iter()
        .map(|target| graph.add_callee(target.clone()))
        .collect();
    // Call-site permissions travel with the copied direct calls. The callee's
    // own `arg_permissions` stay behind: its protector ends at this seam
    // (inlining v1), so no spliced access gains a scope it did not have.
    for target in &source.callees {
        if let Some(permissions) = source.callee_permissions.get(&target.symbol) {
            graph
                .callee_permissions
                .entry(target.symbol.clone())
                .or_insert_with(|| permissions.clone());
        }
    }
    let call_abis: Vec<_> = source
        .call_abis
        .iter()
        .map(|abi| {
            let index = graph.call_abis.len() as u32;
            graph.call_abis.push(abi.clone());
            index
        })
        .collect();
    let mut mapping = vec![NodeId::NONE; source.len()];
    let mut control = graph.resolve(graph.control(call));
    let mut effect = graph.resolve(graph.input(call, 0));
    let mut observed = Vec::new();
    for value in arguments {
        let (next_control, next_effect) = require_defined(graph, control, effect, value);
        control = next_control;
        effect = next_effect;
        // Keep observation fixed, but expose the original value structure to
        // seam folding; RequireDefined is not an optimization barrier.
        observed.push(value);
    }
    let mut slot_nodes = Vec::new();
    for &id in &ids {
        if source.op(id) == Opcode::LocalSlot {
            let slot = graph.add_node(
                Opcode::LocalSlot,
                NodeId::NONE,
                &[],
                locals[&source.payload(id)],
                imported[&id].clone(),
            );
            mapping[id.index()] = slot;
            slot_nodes.push(slot);
            dirty.push(slot);
            (control, effect) = lifetime(graph, Opcode::SlotLive, control, effect, slot);
        }
    }
    let mut nested = Vec::new();
    let mut cloned_nodes = slot_nodes.len();
    for &id in &ids {
        if id == source.start || matches!(source.op(id), Opcode::Return | Opcode::LocalSlot) {
            continue;
        }
        if source.op(id) == Opcode::Projection && source.input(id, 0) == source.start {
            mapping[id.index()] = match source.payload(id) {
                0 => control,
                1 => effect,
                index => observed[index as usize - 2],
            };
            continue;
        }
        let payload = match source.op(id) {
            Opcode::Const | Opcode::ConstRepr => {
                let index = graph.constants.len() as u32;
                graph.constants.push(source.constant_value(id));
                index
            }
            Opcode::Call => callees[source.payload(id) as usize],
            Opcode::CallIndirect => call_abis[source.payload(id) as usize],
            Opcode::SymbolAddress => symbols[source.payload(id) as usize],
            Opcode::VariantSwitch => {
                let index = graph.variant_switches.len() as u32;
                graph
                    .variant_switches
                    .push(source.variant_switches[source.payload(id) as usize].clone());
                index
            }
            Opcode::VariantField | Opcode::VariantFieldAddr => {
                let (variant, field) = source.variant_fields[source.payload(id) as usize];
                graph.intern_variant_field(variant, field)
            }
            Opcode::LoadValue
            | Opcode::StoreValue
            | Opcode::SetVariant
            | Opcode::SlotLive
            | Opcode::SlotDead
            | Opcode::CopyNonOverlapping => {
                let partition = match source.memory_partitions[source.payload(id) as usize] {
                    MemoryPartition::Unknown => MemoryPartition::Unknown,
                    MemoryPartition::Local { local } => MemoryPartition::Local {
                        local: locals[&local],
                    },
                    MemoryPartition::Const { symbol } => MemoryPartition::Const {
                        symbol: symbols[symbol as usize],
                    },
                };
                graph.intern_memory_partition(partition)
            }
            _ => source.payload(id),
        };
        let cloned = graph.add_node(
            source.op(id),
            NodeId::NONE,
            &[],
            payload,
            imported[&id].clone(),
        );
        mapping[id.index()] = cloned;
        cloned_nodes += 1;
        if source.inputs(id).iter().any(|&input| {
            source.op(input) == Opcode::Projection && source.input(input, 0) == source.start
        }) {
            dirty.push(cloned);
        }
        if source.op(id) == Opcode::Call {
            nested.push(cloned);
        }
    }
    let map = |node: NodeId| {
        if node.is_none() {
            NodeId::NONE
        } else {
            mapping[node.index()]
        }
    };
    for &id in &ids {
        if id == source.start
            || source.op(id) == Opcode::Return
            || (source.op(id) == Opcode::Projection && source.input(id, 0) == source.start)
        {
            continue;
        }
        graph.set_inputs(
            map(id),
            &source
                .inputs(id)
                .iter()
                .copied()
                .map(map)
                .collect::<Vec<_>>(),
        );
        graph.set_control(map(id), map(source.control(id)));
    }
    let mut returns = Vec::new();
    for ret in ids
        .iter()
        .copied()
        .filter(|&n| source.op(n) == Opcode::Return)
    {
        let mut control = map(source.control(ret));
        let mut effect = map(source.input(ret, 0));
        let value = if let Some(&value) = source.inputs(ret).get(1) {
            let value = map(value);
            let (next_control, next_effect) = require_defined(graph, control, effect, value);
            control = next_control;
            effect = next_effect;
            Some(value)
        } else {
            None
        };
        for &slot in &slot_nodes {
            (control, effect) = lifetime(graph, Opcode::SlotDead, control, effect, slot);
        }
        returns.push((control, effect, value));
    }
    let (control, effect, value) = if returns.len() == 1 {
        returns[0]
    } else {
        let jumps: Vec<_> = returns
            .iter()
            .map(|(control, _, _)| {
                graph.add_node(Opcode::Jump, *control, &[], 0, ValueType::Control)
            })
            .collect();
        let region = graph.add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
        let mut effects = vec![region];
        effects.extend(returns.iter().map(|r| r.1));
        let effect = graph.add_node(Opcode::Phi, NodeId::NONE, &effects, 0, ValueType::Effect);
        let value = if returns.iter().all(|r| r.2.is_some()) {
            let mut values = vec![region];
            values.extend(returns.iter().map(|r| r.2.unwrap()));
            Some(graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &values,
                0,
                expected_result.clone().expect("preflight result type"),
            ))
        } else if returns.iter().all(|r| r.2.is_none()) {
            None
        } else {
            unreachable!("preflight consistent returns")
        };
        dirty.push(region);
        dirty.push(effect);
        if let Some(value) = value {
            dirty.push(value);
        }
        (region, effect, value)
    };
    for projection in projections {
        let replacement = match graph.payload(projection) {
            0 => control,
            1 => effect,
            2 => value.expect("preflight returning call"),
            _ => unreachable!("verified projection"),
        };
        graph.replace(projection, replacement);
    }
    graph.remove_node(call);
    Spliced {
        nested,
        dirty,
        argument_slots,
        cloned_nodes,
    }
}
fn lifetime(
    graph: &mut FunctionGraph,
    op: Opcode,
    control: NodeId,
    effect: NodeId,
    slot: NodeId,
) -> (NodeId, NodeId) {
    // Lifetime boundaries also cover escaping raw pointers; they do not
    // assert the stronger non-escape proof carried by Local accesses.
    let node = graph.add_node(
        op,
        control,
        &[effect, slot],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[node],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[node],
        1,
        ValueType::Effect,
    );
    (control, effect)
}
pub(super) fn argument_slots(graph: &FunctionGraph, arguments: &[NodeId]) -> Vec<NodeId> {
    let mut slots = Vec::new();
    let mut pending = arguments.to_vec();
    let mut seen = HashSet::default();
    while let Some(node) = pending.pop() {
        let node = graph.resolve(node);
        if !seen.insert(node) {
            continue;
        }
        match graph.op(node) {
            Opcode::LocalSlot => slots.push(node),
            Opcode::PointerCast
            | Opcode::FieldAddr
            | Opcode::VariantFieldAddr
            | Opcode::IndexAddr => pending.push(graph.input(node, 0)),
            Opcode::Aggregate | Opcode::Repeat | Opcode::WidePtr | Opcode::InsertField => {
                pending.extend(graph.inputs(node))
            }
            Opcode::Phi => pending.extend(&graph.inputs(node)[1..]),
            _ => {}
        }
    }
    slots
}

pub(super) fn require_defined(
    graph: &mut FunctionGraph,
    control: NodeId,
    effect: NodeId,
    value: NodeId,
) -> (NodeId, NodeId) {
    let fixed = graph.add_node(
        Opcode::RequireDefined,
        control,
        &[effect, value],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[fixed],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[fixed],
        1,
        ValueType::Effect,
    );
    (control, effect)
}
