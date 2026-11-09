//! Validate a call boundary and import its type facts before mutating the caller.

use super::{InlineSkip, types::TypeImport};
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use std::collections::HashMap;

pub(super) struct Plan {
    pub(super) ids: Vec<NodeId>,
    pub(super) imported: HashMap<NodeId, ValueType>,
    pub(super) oracle: kfrc_body::RecordedTypes,
    pub(super) expected_result: Option<ValueType>,
    pub(super) locals: HashMap<u32, u32>,
}
pub(super) fn plan(
    graph: &FunctionGraph,
    call: NodeId,
    source: &FunctionGraph,
    remaining: usize,
) -> Result<Plan, InlineSkip> {
    let ids: Vec<_> = source.ids().collect();
    for data in &source.data_objects {
        // A provider can expose another CGU's graph. Duplicating an object
        // with local linkage changes identity and potentially mutable state.
        // A symbol-owner contract must export a shared definition first.
        if data.binding == kfrc_body::DataBinding::Local {
            return Err(InlineSkip::DataConflict);
        }
        if let Some(old) = graph.data_objects.iter().find(|d| d.symbol == data.symbol) {
            if old != data {
                return Err(InlineSkip::DataConflict);
            }
        }
    }
    let returns: Vec<_> = ids
        .iter()
        .copied()
        .filter(|&n| source.op(n) == Opcode::Return)
        .collect();
    let valued = returns
        .iter()
        .filter(|&&n| source.inputs(n).len() > 1)
        .count();
    if valued != 0 && valued != returns.len() {
        return Err(InlineSkip::Unsupported);
    }
    let local_nodes: Vec<_> = ids
        .iter()
        .copied()
        .filter(|&n| source.op(n) == Opcode::LocalSlot)
        .collect();
    let next_local = if local_nodes.is_empty() {
        0
    } else {
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::LocalSlot)
            .map(|n| graph.payload(n))
            .max()
            .map_or(Some(0), |id| id.checked_add(1))
            .ok_or(InlineSkip::Budget)?
    };
    let mut locals = HashMap::new();
    for (index, node) in local_nodes.iter().enumerate() {
        let local = next_local
            .checked_add(u32::try_from(index).map_err(|_| InlineSkip::Budget)?)
            .ok_or(InlineSkip::Budget)?;
        locals.insert(source.payload(*node), local);
    }
    let cloned = ids
        .iter()
        .filter(|&&n| {
            n != source.start
                && source.op(n) != Opcode::Return
                && !(source.op(n) == Opcode::Projection && source.input(n, 0) == source.start)
        })
        .count();
    let extra = (graph.inputs(call).len() - 1) * 3
        + valued * 3
        + local_nodes.len() * 3 * (returns.len() + 1)
        + if returns.len() > 1 {
            returns.len() + 2 + usize::from(valued > 0)
        } else {
            0
        };
    if cloned.saturating_add(extra) > remaining {
        return Err(InlineSkip::Budget);
    }
    let mut oracle = graph.oracle.clone();
    let mut types = TypeImport::new(&source.oracle, &mut oracle);
    let imported: HashMap<_, _> = ids
        .iter()
        .map(|&id| Ok((id, types.value_type(source.value_type(id))?)))
        .collect::<Result<_, InlineSkip>>()?;
    types.finish()?;
    let ValueType::Tuple(start_types) = &imported[&source.start] else {
        return Err(InlineSkip::Unsupported);
    };
    let arguments: Vec<_> = graph.inputs(call)[1..]
        .iter()
        .map(|n| graph.resolve(*n))
        .collect();
    if arguments.len() + 2 != start_types.len()
        || arguments
            .iter()
            .zip(&start_types[2..])
            .any(|(arg, ty)| graph.value_type(*arg) != ty)
    {
        return Err(InlineSkip::TypeConflict);
    }
    let ValueType::Tuple(result_types) = graph.value_type(call).clone() else {
        return Err(InlineSkip::Unsupported);
    };
    let expected_result = result_types.get(2).cloned();
    for ret in ids
        .iter()
        .copied()
        .filter(|&n| source.op(n) == Opcode::Return)
    {
        if let Some(&value) = source.inputs(ret).get(1) {
            if expected_result.as_ref() != Some(&imported[&value]) {
                return Err(InlineSkip::TypeConflict);
            }
        } else if graph.find_projection(call, 2).is_some() {
            return Err(InlineSkip::Unsupported);
        }
    }
    Ok(Plan {
        ids,
        imported,
        oracle,
        expected_result,
        locals,
    })
}
