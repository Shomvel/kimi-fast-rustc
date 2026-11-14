//! Unique structurally empty Rust values shared by Body building and R specialization.
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{TyId, TypeOracle, TypeShape};

/// Construct a value using semantic fields and inhabitedness, never layout bytes.
/// Ambiguous or uninhabited enums cannot be guessed from an ignored ABI result.
pub fn empty_value(graph: &mut FunctionGraph, ty: TyId) -> Result<NodeId, String> {
    if graph.oracle.semantic().maybe_uninit_inner(ty).is_some() {
        return Ok(graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Rust(ty)));
    }
    match graph.oracle.shape(ty) {
        TypeShape::Unit | TypeShape::FunctionItem { .. } => Ok(graph.add_constant(0, ty)),
        TypeShape::Array { element, length } => {
            let (element, length) = (*element, *length);
            // Empty arrays contain no element to initialize or observe.
            // For a nonempty ZST array construct its element once, retaining
            // the logical repetition until physical lowering.
            let value = if length == 0 {
                graph.add_node(
                    Opcode::Undefined,
                    NodeId::NONE,
                    &[],
                    0,
                    ValueType::Rust(element),
                )
            } else {
                empty_value(graph, element)?
            };
            Ok(graph.add_node(
                Opcode::Repeat,
                NodeId::NONE,
                &[value],
                0,
                ValueType::Rust(ty),
            ))
        }
        TypeShape::Tuple(_) | TypeShape::Adt { .. } => {
            let variant = if graph.oracle.semantic().is_enum(ty) {
                match graph.oracle.semantic().inhabited_variants(ty) {
                    Some([variant]) => Some(*variant),
                    None if graph.oracle.semantic().variant_count(ty) == Some(1) => Some(0),
                    _ => {
                        return Err(
                            "zero-sized enum constant requires a unique inhabited variant".into(),
                        );
                    }
                }
            } else {
                None
            };
            let fields = graph
                .oracle
                .semantic()
                .field_types(ty, variant)
                .ok_or_else(|| "missing semantic fields for empty value".to_string())?;
            let mut values = Vec::with_capacity(fields.len());
            for field in fields {
                values.push(empty_value(graph, field)?);
            }
            Ok(graph.add_node(
                if variant.is_some() {
                    Opcode::MakeVariant
                } else {
                    Opcode::Aggregate
                },
                NodeId::NONE,
                &values,
                variant.unwrap_or(0),
                ValueType::Rust(ty),
            ))
        }
        _ => Err("ignored or zero-sized value is not structurally empty".into()),
    }
}
