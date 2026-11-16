//! Reuse an SSA length through one executed, complete wide-pointer conversion.
//!
//! This is a representation certificate, not a generic field invariant. The
//! fixed conversion still observes the entire source and decodes the entire
//! target at its original control/effect position, even if its value is unused.

use kfrc_body::{FieldLayout, IntegerWidth, ScalarValidity, TyId, TypeOracle, TypeShape};

use crate::{FunctionGraph, NodeId, Opcode, ValueType};

const PROOF_BUDGET: usize = 256;

pub(super) fn through_transmute(
    graph: &mut FunctionGraph,
    field: NodeId,
    converted: NodeId,
) -> Option<NodeId> {
    if graph.payload(field) != 1
        || !std::env::var_os("KFRC_REPR_WIDEPTR_METADATA").is_some_and(|value| value == "1")
    {
        return None;
    }
    let (source, metadata) = wideptr_metadata_source(graph, converted)?;
    if graph.rust_type(field) != Some(metadata) {
        return None;
    }
    // No fixed node or edge is bypassed. In particular graph compaction roots
    // every fixed operation, including TransmuteValue without a value user.
    // Field reads the existing SSA snapshot; it never reloads its source.
    Some(graph.add_node(
        Opcode::Field,
        NodeId::NONE,
        &[source],
        1,
        ValueType::Rust(metadata),
    ))
}

/// Certify only metadata field 1 of one complete, already executed conversion.
/// Consumers retain the fixed conversion and all whole-value observations;
/// this fact neither validates the data pointer nor authorizes ordinary casts.
/// The certificate remains available when the opt-in Field rewrite is disabled.
pub(crate) fn wideptr_metadata_source(
    graph: &FunctionGraph,
    converted: NodeId,
) -> Option<(NodeId, TyId)> {
    let converted = graph.resolve(converted);
    if graph.op(converted) != Opcode::Projection || graph.payload(converted) != 2 {
        return None;
    }
    let conversion = graph.resolve(*graph.inputs(converted).first()?);
    if graph.op(conversion) != Opcode::TransmuteValue || graph.control(conversion).is_none() {
        return None;
    }
    let source = graph.resolve(*graph.inputs(conversion).get(1)?);
    let from = LengthLayout::of(graph, graph.rust_type(source)?)?;
    let to = LengthLayout::of(graph, graph.rust_type(converted)?)?;
    if from != to {
        return None;
    }
    let execution = Execution::before(graph, conversion)?;
    if !execution.stable_snapshot(graph, source) {
        return None;
    }
    Some((source, from.metadata.ty))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct LengthLayout {
    metadata: FieldLayout,
    data_offset: u64,
    component_size: u64,
    size: u64,
}

impl LengthLayout {
    fn of(graph: &FunctionGraph, ty: TyId) -> Option<Self> {
        let types = &graph.oracle;
        let semantic = types.semantic();
        let pointee = match semantic.shape(ty) {
            TypeShape::Reference { pointee, .. } | TypeShape::RawPointer { pointee, .. } => {
                *pointee
            }
            _ => return None,
        };
        // A dyn vtable and arbitrary aggregate pair are never slice lengths.
        if !matches!(
            semantic.shape(pointee),
            TypeShape::Slice { .. } | TypeShape::Str
        ) || semantic.is_union(ty)
            || semantic.maybe_uninit_inner(ty).is_some()
            || semantic.field_count(ty, None) != Some(2)
        {
            return None;
        }
        let layout = types.aggregates.get(&ty)?;
        let [data, metadata] = layout.fields.as_slice() else {
            return None;
        };
        if !layout.variants.is_empty()
            || semantic.field_type(ty, None, 0) != Some(data.ty)
            || semantic.field_type(ty, None, 1) != Some(metadata.ty)
            || semantic.is_union(data.ty)
            || semantic.is_union(metadata.ty)
            || semantic.maybe_uninit_inner(data.ty).is_some()
            || semantic.maybe_uninit_inner(metadata.ty).is_some()
            || !matches!(semantic.shape(data.ty), TypeShape::RawPointer { .. })
            || semantic.thin_pointee(data.ty).is_none()
            || !matches!(
                semantic.shape(metadata.ty),
                TypeShape::Int {
                    width: IntegerWidth::Pointer,
                    signed: false
                }
            )
            || types.scalar_validity(metadata.ty).is_some()
        {
            return None;
        }
        let size = types.size_align(ty)?.size;
        let component_size = types.size_align(metadata.ty)?.size;
        let data_size = types.size_align(data.ty)?.size;
        let data_end = data.offset.checked_add(data_size)?;
        let metadata_end = metadata.offset.checked_add(component_size)?;
        if component_size == 0
            || component_size > 8
            || data_size != component_size
            || data_end > size
            || metadata_end > size
            || !(data_end <= metadata.offset || metadata_end <= data.offset)
            || data_size.checked_add(component_size)? != size
        {
            return None;
        }
        // The fixed decode retains reference/data-pointer validity. Never
        // project through an extra restriction on the metadata scalar/lane.
        match types.scalar_validity(ty) {
            None => {}
            Some(ScalarValidity::Pair {
                second: None,
                second_offset,
                ..
            }) if data.offset == 0 && second_offset == metadata.offset => {}
            _ => return None,
        }
        Some(Self {
            metadata: *metadata,
            data_offset: data.offset,
            component_size,
            size,
        })
    }
}

/// The first version excludes merges and loops rather than attempting a
/// cross-iteration fact. A fixed snapshot must occur on both predecessor
/// spines of this conversion; entry values and pure expressions of these
/// snapshots cannot change between the conversion and its original uses.
struct Execution {
    ancestors: Vec<(NodeId, bool)>,
}

impl Execution {
    fn before(graph: &FunctionGraph, conversion: NodeId) -> Option<Self> {
        let mut ancestors = Vec::new();
        let mut control = graph.resolve(graph.control(conversion));
        let mut steps = 0;
        loop {
            if steps == PROOF_BUDGET
                || control.is_none()
                || ancestors.iter().any(|&(node, _)| node == control)
                || matches!(graph.op(control), Opcode::Region | Opcode::Phi)
            {
                return None;
            }
            steps += 1;
            ancestors.push((control, false));
            if control == graph.start {
                break;
            }
            control = graph.resolve(if graph.op(control) == Opcode::Projection {
                *graph.inputs(control).first()?
            } else {
                graph.control(control)
            });
        }
        let mut effect = graph.resolve(*graph.inputs(conversion).first()?);
        loop {
            if steps == PROOF_BUDGET
                || graph.op(effect) != Opcode::Projection
                || graph.payload(effect) != 1
                || graph.value_type(effect) != &ValueType::Effect
            {
                return None;
            }
            steps += 1;
            let event = graph.resolve(*graph.inputs(effect).first()?);
            let Some((_, seen)) = ancestors.iter_mut().find(|(node, _)| *node == event) else {
                return None;
            };
            if *seen {
                return None;
            }
            *seen = true;
            if event == graph.start {
                return Some(Self { ancestors });
            }
            if !graph.op(event).is_fixed() {
                return None;
            }
            effect = graph.resolve(*graph.inputs(event).first()?);
        }
    }

    fn stable_snapshot(&self, graph: &FunctionGraph, source: NodeId) -> bool {
        let mut pending = vec![source];
        let mut seen = Vec::new();
        let mut steps = 0;
        while let Some(node) = pending.pop() {
            if steps == PROOF_BUDGET {
                return false;
            }
            steps += 1;
            let node = graph.resolve(node);
            if seen.contains(&node) {
                continue;
            }
            seen.push(node);
            if !graph.control(node).is_none() || graph.op(node) == Opcode::Phi {
                return false;
            }
            if graph.op(node) == Opcode::Projection {
                let Some(&producer) = graph.inputs(node).first() else {
                    return false;
                };
                let producer = graph.resolve(producer);
                if graph.payload(node) < 2 || !self.ancestors.contains(&(producer, true)) {
                    return false;
                }
            } else if graph.op(node).is_fixed() {
                return false;
            } else {
                if graph.inputs(node).len() > PROOF_BUDGET - steps {
                    return false;
                }
                pending.extend_from_slice(graph.inputs(node));
            }
        }
        true
    }
}
