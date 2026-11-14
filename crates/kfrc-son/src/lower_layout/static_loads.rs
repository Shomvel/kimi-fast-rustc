//! Resolve fixed scalar reads from known immutable data after publishing L.
//! Full pointer relocations retain their symbol identity; partial relocation
//! reads, mutable/TLS objects and unknown address paths stay loads.
use super::*;
use crate::analysis::uses::{UseKind, Uses};
use kfrc_body::{DataBinding, DataObject, DataRelocation, Mutability};

fn immutable_data<'a>(graph: &'a FunctionGraph, symbol: &str) -> Option<&'a DataObject> {
    let object = graph.data_object(symbol)?;
    (!object.writable
        && !object.thread_local
        && !matches!(object.binding, DataBinding::Global { hidden: false }))
    .then_some(object)
}

fn address_word(graph: &FunctionGraph, ty: TyId) -> bool {
    graph
        .oracle
        .size_align(ty)
        .is_some_and(|layout| layout.size == 8)
        && match graph.oracle.shape(ty) {
            TypeShape::Int { width, .. } => {
                width.bits(64) == 64 && graph.oracle.scalar_validity(ty).is_none()
            }
            TypeShape::RawPointer { .. } | TypeShape::Reference { .. } => {
                graph.oracle.semantic().thin_pointee(ty).is_some()
            }
            _ => false,
        }
}

fn result_type(graph: &FunctionGraph, node: NodeId) -> Option<TyId> {
    let ValueType::Tuple(fields) = graph.value_type(node) else {
        return None;
    };
    let [ValueType::Control, ValueType::Effect, ValueType::Rust(ty)] = fields.as_slice() else {
        return None;
    };
    Some(*ty)
}

fn overlaps(relocation: &DataRelocation, offset: usize, end: usize) -> bool {
    relocation
        .offset
        .checked_add(8)
        .is_none_or(|relocation_end| {
            relocation.offset < end as u64 && (offset as u64) < relocation_end
        })
}

fn relocation<'a>(
    graph: &'a FunctionGraph,
    load: NodeId,
    remaining: &mut usize,
) -> Option<&'a DataRelocation> {
    if graph.op(load) != Opcode::Load || !address_word(graph, result_type(graph, load)?) {
        return None;
    }
    let (symbol, offset) = address(graph, graph.input(load, 1), remaining)?;
    let object = immutable_data(graph, symbol)?;
    let end = offset.checked_add(8)?;
    object.bytes.get(offset..end)?;
    let mut matches = object
        .relocations
        .iter()
        .filter(|r| overlaps(r, offset, end));
    let selected = matches.next()?;
    if selected.offset != offset as u64 || matches.next().is_some() {
        return None;
    }
    // An ordinary SymbolAddress cannot represent TLS. Restrict targets to
    // known non-preemptible data as well; external/function cases stay loads.
    let target = graph.data_object(&selected.target)?;
    (!target.thread_local && !matches!(target.binding, DataBinding::Global { hidden: false }))
        .then_some(selected)
}

fn address<'a>(
    graph: &'a FunctionGraph,
    mut node: NodeId,
    remaining: &mut usize,
) -> Option<(&'a str, usize)> {
    let mut offset = 0i128;
    // All recursive relocation/address transitions share this query's bound.
    while *remaining != 0 {
        *remaining -= 1;
        node = graph.resolve(node);
        if !graph.control(node).is_none() {
            return None;
        }
        match graph.op(node) {
            Opcode::SymbolAddress => {
                let (symbol, addend) = graph.symbols.get(graph.payload(node) as usize)?;
                offset = offset.checked_add(i128::from(*addend))?;
                return Some((symbol, usize::try_from(offset).ok()?));
            }
            Opcode::PointerCast => node = *graph.inputs(node).first()?,
            Opcode::IntCast
                if graph.payload(node) == 0
                    && address_word(graph, graph.rust_type(node)?)
                    && address_word(graph, graph.rust_type(graph.input(node, 0))?) =>
            {
                node = graph.input(node, 0);
            }
            Opcode::Projection if graph.payload(node) == 2 => {
                let relocation = relocation(graph, graph.input(node, 0), remaining)?;
                offset = offset.checked_add(i128::from(relocation.addend))?;
                return Some((&relocation.target, usize::try_from(offset).ok()?));
            }
            Opcode::PtrOffset => {
                let [base, displacement] = graph.inputs(node) else {
                    return None;
                };
                let displacement = graph.resolve(*displacement);
                if graph.op(displacement) != Opcode::Const {
                    return None;
                }
                let ty = graph.rust_type(displacement)?;
                let TypeShape::Int { width, signed } = graph.oracle.shape(ty) else {
                    return None;
                };
                let bits = width.bits(64);
                let raw = graph.constant_value(displacement);
                if bits > 64 || raw >= (1u128 << bits) {
                    return None;
                }
                let displacement = if *signed && raw & (1u128 << (bits - 1)) != 0 {
                    raw as i128 - (1i128 << bits)
                } else {
                    raw as i128
                };
                offset = offset.checked_add(displacement)?;
                node = *base;
            }
            _ => return None,
        }
    }
    None
}

enum FoldedValue {
    Scalar(TyId, u128),
    Symbol(TyId, String, i64),
}

fn value(graph: &FunctionGraph, node: NodeId) -> Option<FoldedValue> {
    let ty = result_type(graph, node)?;
    let mut remaining = 32;
    let bytes = match graph.oracle.shape(ty) {
        TypeShape::Int { width, .. } if width.bits(64) <= 64 => usize::from(width.bits(64) / 8),
        TypeShape::Bool => 1,
        TypeShape::Char => 4,
        TypeShape::RawPointer { .. } | TypeShape::Reference { .. } if address_word(graph, ty) => 8,
        _ => return None,
    };
    if graph.oracle.size_align(ty)?.size != bytes as u64
        || matches!(graph.oracle.shape(ty), TypeShape::Int { .. })
            && graph.oracle.scalar_validity(ty).is_some()
    {
        return None;
    }
    if bytes == 8
        && let Some(relocation) = relocation(graph, node, &mut remaining)
    {
        return Some(FoldedValue::Symbol(
            ty,
            relocation.target.clone(),
            relocation.addend,
        ));
    }
    // Restart the bounded address query after checking the optional pointer
    // case. At most two 32-transition queries are made for any candidate load.
    remaining = 32;
    let (symbol, offset) = address(graph, graph.input(node, 1), &mut remaining)?;
    let object = immutable_data(graph, symbol)?;
    let end = offset.checked_add(bytes)?;
    let data = object.bytes.get(offset..end)?;
    // A pointer-sized relocation replaces all eight bytes, including partial
    // reads. Its link/load-time contents cannot be read from the template.
    if object.relocations.iter().any(|r| overlaps(r, offset, end))
        || matches!(
            graph.oracle.shape(ty),
            TypeShape::RawPointer { .. } | TypeShape::Reference { .. }
        )
    {
        return None;
    }
    let bits = data.iter().enumerate().fold(0u128, |bits, (index, byte)| {
        bits | (u128::from(*byte) << (index * 8))
    });
    // Malformed typed values must not become well-typed constants.
    if matches!(graph.oracle.shape(ty), TypeShape::Bool) && bits > 1
        || matches!(graph.oracle.shape(ty), TypeShape::Char)
            && char::from_u32(bits as u32).is_none()
    {
        return None;
    }
    Some(FoldedValue::Scalar(ty, bits))
}

pub(super) fn run(graph: &mut FunctionGraph) -> usize {
    debug_assert_eq!(graph.level, GraphLevel::Layout);
    graph.resolve_edges();
    let replacements: Vec<_> = graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::Load)
        .filter_map(|node| value(graph, node).map(|value| (node, value)))
        .collect();
    if replacements.is_empty() {
        return 0;
    }
    let uses = Uses::new(graph);
    let mut folded = 0;
    for (load, folded_value) in replacements {
        // Verification guarantees tuple results are consumed by projections.
        // Check the complete use shape before bypassing the original effect.
        if uses.of(load).iter().any(|usage| {
            usage.kind != UseKind::Operand(0)
                || graph.op(usage.user) != Opcode::Projection
                || graph.payload(usage.user) > 2
        }) {
            continue;
        }
        let value = match folded_value {
            FoldedValue::Scalar(ty, bits) => graph.add_constant(bits, ty),
            FoldedValue::Symbol(ty, symbol, addend) => {
                let pointer = if matches!(graph.oracle.shape(ty), TypeShape::Int { .. }) {
                    graph.oracle.intern(
                        TypeShape::RawPointer {
                            mutability: Mutability::Not,
                            pointee: ty,
                        },
                        Some(SizeAlign { size: 8, align: 8 }),
                    )
                } else {
                    ty
                };
                let payload = graph.symbols.len() as u32;
                graph.symbols.push((symbol, addend));
                let address = graph.add_node(
                    Opcode::SymbolAddress,
                    NodeId::NONE,
                    &[],
                    payload,
                    ValueType::Rust(pointer),
                );
                if pointer == ty {
                    address
                } else {
                    graph.add_node(
                        Opcode::IntCast,
                        NodeId::NONE,
                        &[address],
                        0,
                        ValueType::Rust(ty),
                    )
                }
            }
        };
        for usage in uses.of(load) {
            let replacement = match graph.payload(usage.user) {
                0 => graph.control(load),
                1 => graph.input(load, 0),
                2 => value,
                _ => unreachable!(),
            };
            graph.replace(usage.user, replacement);
        }
        graph.remove_node(load);
        folded += 1;
    }
    graph.resolve_edges();
    folded
}

#[cfg(test)]
#[path = "static_loads_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "static_relocations_tests.rs"]
mod relocation_tests;
