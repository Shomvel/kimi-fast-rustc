//! Conservative pointer-parameter capture and access inference.
use super::*;
use crate::analysis::uses::UseKind;
use kfrc_body::{TypeOracle, TypeShape};

fn pointer_type(graph: &FunctionGraph, ty: &ValueType) -> bool {
    matches!(ty, ValueType::Rust(ty) if matches!(graph.oracle.semantic().shape(*ty),
        TypeShape::Reference { .. } | TypeShape::RawPointer { .. } | TypeShape::PlaceAddress { .. }))
}

pub(super) fn argument_facts(
    graph: &FunctionGraph,
    parameter: usize,
    ty: &ValueType,
    uses: &Uses,
    bodies: &BTreeMap<String, FunctionGraph>,
) -> ArgumentFacts {
    let mut result = ArgumentFacts {
        attrs: ArgumentAttrs::UNKNOWN,
        calls: Vec::new(),
    };
    if !pointer_type(graph, ty) {
        return result;
    }
    result.attrs = ArgumentAttrs::UNUSED;
    let mut queue: Vec<_> = uses
        .of(graph.start)
        .iter()
        .filter_map(|usage| {
            let node = usage.user;
            (usage.kind == UseKind::Operand(0)
                && graph.op(node) == Opcode::Projection
                && graph.payload(node) as usize == parameter + 2)
                .then_some(node)
        })
        .collect();
    let mut visited = BTreeSet::new();
    while let Some(value) = queue.pop() {
        if !visited.insert(value) {
            continue;
        }
        for usage in uses.of(value) {
            let node = usage.user;
            let UseKind::Operand(index) = usage.kind else {
                result.attrs = ArgumentAttrs::UNKNOWN;
                continue;
            };
            match graph.op(node) {
                Opcode::PointerCast
                | Opcode::FieldAddr
                | Opcode::VariantFieldAddr
                | Opcode::IndexAddr
                | Opcode::WidePtr
                    if index == 0 =>
                {
                    queue.push(node)
                }
                Opcode::Field
                    if index == 0
                        && graph.payload(node) == 0
                        && pointer_type(graph, graph.value_type(node)) =>
                {
                    queue.push(node)
                }
                Opcode::Phi if index > 0 && pointer_type(graph, graph.value_type(node)) => {
                    queue.push(node)
                }
                Opcode::LoadValue | Opcode::Load if index == 1 => {
                    result.attrs.memory = result.attrs.memory.max(MemoryEffects::ReadOnly);
                }
                Opcode::StoreValue
                | Opcode::Store
                | Opcode::SetVariant
                | Opcode::SlotLive
                | Opcode::SlotDead
                    if index == 1 =>
                {
                    result.attrs.memory = MemoryEffects::Unknown;
                }
                Opcode::CopyNonOverlapping if index == 1 => {
                    result.attrs.memory = result.attrs.memory.max(MemoryEffects::ReadOnly);
                }
                Opcode::CopyNonOverlapping if index == 2 => {
                    result.attrs.memory = MemoryEffects::Unknown
                }
                Opcode::RequireDefined if index == 1 => {}
                Opcode::Call if index > 0 => {
                    if let Some(callee) = direct_callee(graph, node, bodies) {
                        result.calls.push((callee.to_owned(), index - 1));
                    } else {
                        result.attrs = ArgumentAttrs::UNKNOWN;
                    }
                }
                _ => result.attrs = ArgumentAttrs::UNKNOWN,
            }
        }
    }
    result
}
