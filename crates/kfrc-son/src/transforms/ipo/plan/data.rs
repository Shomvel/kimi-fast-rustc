//! Copy only data reachable from the nodes being inlined. Simplifying a graph
//! does not prune its data table; dead panic locations must not block a pure
//! wrapper. Relocations and memory proof partitions are references too.
use crate::{FunctionGraph, MemoryPartition, NodeId, Opcode};
use kfrc_body::{DataBinding, fx_hash::HashMap};

pub(super) fn referenced(graph: &FunctionGraph, ids: &[NodeId]) -> Vec<usize> {
    if graph.data_objects.is_empty() {
        return Vec::new();
    }
    let objects: HashMap<_, _> = graph
        .data_objects
        .iter()
        .enumerate()
        .map(|(i, object)| (object.symbol.as_str(), i))
        .collect();
    let mut pending = Vec::new();
    let mut visited = vec![false; graph.data_objects.len()];
    // Keep exported definitions even without a local use. This is an import
    // filter, not a proof of whole-program deadness or symbol reachability.
    for (index, object) in graph.data_objects.iter().enumerate() {
        if object.binding != DataBinding::Local {
            pending.push(index);
        }
    }
    for &id in ids {
        let symbol = match graph.op(id) {
            Opcode::SymbolAddress | Opcode::ThreadLocalAddress => {
                Some(graph.symbols[graph.payload(id) as usize].0.as_str())
            }
            Opcode::Call => Some(graph.callees[graph.payload(id) as usize].symbol.as_str()),
            Opcode::LoadValue
            | Opcode::StoreValue
            | Opcode::SetVariant
            | Opcode::SlotLive
            | Opcode::SlotDead
            | Opcode::CopyNonOverlapping => {
                match graph.memory_partitions[graph.payload(id) as usize] {
                    MemoryPartition::Const { symbol } => {
                        Some(graph.symbols[symbol as usize].0.as_str())
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(index) = symbol.and_then(|name| objects.get(name)) {
            pending.push(*index);
        }
    }
    while let Some(index) = pending.pop() {
        if visited[index] {
            continue;
        }
        visited[index] = true;
        for relocation in &graph.data_objects[index].relocations {
            if let Some(target) = objects.get(relocation.target.as_str()) {
                pending.push(*target);
            }
        }
    }
    visited
        .iter()
        .enumerate()
        .filter_map(|(index, &live)| live.then_some(index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValueType;
    use kfrc_body::{DataObject, DataRelocation, FunctionAbi, RecordedTypes};

    #[test]
    fn follows_relocations_through_cycles_and_keeps_exported_roots() {
        let mut g = FunctionGraph::new(
            "data_closure",
            FunctionAbi {
                args: vec![],
                ret: kfrc_body::PassMode::Ignore,
                c_variadic: false,
            },
            RecordedTypes::new(),
        );
        for name in ["root", "child", "orphan", "export", "export_child"] {
            g.data_objects.push(DataObject {
                symbol: name.into(),
                bytes: vec![0; 8],
                align: 8,
                ..Default::default()
            });
        }
        for (source, target) in [(0, "child"), (1, "root"), (3, "export_child")] {
            g.data_objects[source].relocations.push(DataRelocation {
                offset: 0,
                target: target.into(),
                addend: 0,
            });
        }
        g.data_objects[3].binding = DataBinding::Global { hidden: false };
        g.symbols.push(("root".into(), 0));
        let address = g.add_node(
            Opcode::SymbolAddress,
            NodeId::NONE,
            &[],
            0,
            ValueType::Control,
        );
        assert_eq!(referenced(&g, &[address]), vec![0, 1, 3, 4]);
        assert_eq!(referenced(&g, &[]), vec![3, 4]);
        g.memory_partitions
            .push(MemoryPartition::Const { symbol: 0 });
        let load = g.add_node(
            Opcode::LoadValue,
            NodeId::NONE,
            &[],
            (g.memory_partitions.len() - 1) as u32,
            ValueType::Control,
        );
        assert_eq!(referenced(&g, &[load]), vec![0, 1, 3, 4]);
    }
}
