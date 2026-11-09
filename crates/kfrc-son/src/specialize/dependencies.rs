//! Definition-only dependency indexing. This cost is not repeated for every T.
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{TyId, TypeOracle, TypeShape};
use std::collections::HashSet;

pub(super) fn parameter_dependent_nodes(graph: &FunctionGraph) -> HashSet<NodeId> {
    // Start with direct parameters, then propagate through the structural type
    // graph to a fixed point. This handles recursive nominal types without
    // accidentally caching an incomplete answer at a cycle backedge.
    let count = graph.oracle.table.len();
    let mut dependent = vec![false; count];
    let mut children = vec![Vec::new(); count];
    for index in 0..count {
        let ty = TyId(index as u32);
        match graph.oracle.shape(ty) {
            TypeShape::Param { .. } => dependent[index] = true,
            TypeShape::Tuple(fields) => children[index].extend(fields),
            TypeShape::Array { element, .. } | TypeShape::Slice { element } => {
                children[index].push(*element)
            }
            TypeShape::Reference { pointee, .. }
            | TypeShape::RawPointer { pointee, .. }
            | TypeShape::PlaceAddress { pointee } => children[index].push(*pointee),
            _ => {}
        }
        if let Some(aggregate) = graph.oracle.semantic_aggregate(ty) {
            children[index].extend(&aggregate.fields);
            for fields in &aggregate.variants {
                children[index].extend(fields);
            }
        }
    }
    loop {
        let mut changed = false;
        for index in 0..count {
            if !dependent[index] && children[index].iter().any(|ty| dependent[ty.0 as usize]) {
                dependent[index] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    fn value(ty: &ValueType, dependent: &[bool]) -> bool {
        match ty {
            ValueType::Rust(ty) | ValueType::LayoutValue(ty) => dependent[ty.0 as usize],
            ValueType::Tuple(fields) => fields.iter().any(|ty| value(ty, dependent)),
            _ => false,
        }
    }
    graph
        .ids()
        .filter(|node| {
            // A generic parameter in Start's tuple does not make every other
            // argument (or its control/effect projection) type-dependent.
            if graph.op(*node) == Opcode::Start {
                return false;
            }
            if graph.op(*node) == Opcode::Projection {
                return value(graph.value_type(*node), &dependent);
            }
            value(graph.value_type(*node), &dependent)
                || graph
                    .inputs(*node)
                    .iter()
                    .any(|input| value(graph.value_type(*input), &dependent))
        })
        .collect()
}
