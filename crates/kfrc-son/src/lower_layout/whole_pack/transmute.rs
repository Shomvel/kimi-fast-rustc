//! A narrow bridge from scalarized S arrays to the complete L pack certificate.
use super::*;

pub(super) fn pack(
    graph: &mut FunctionGraph,
    source: &Value,
    target: TyId,
    max_bytes: u64,
) -> Option<Value> {
    let bytes = u64::from(pack::unsigned(graph, target)? / 8);
    let TypeShape::Array { element, length } = *graph.oracle.shape(source.ty) else {
        return None;
    };
    if !matches!(bytes, 2 | 4 | 8)
        || bytes > max_bytes
        || length != bytes
        || pack::unsigned(graph, element)? != 8
        || graph.oracle.size_align(source.ty)?.size != bytes
        || graph.oracle.scalar_validity(source.ty).is_some()
        || graph.oracle.semantic().is_union(source.ty)
        || graph.oracle.semantic().is_vector(source.ty)
    {
        return None;
    }
    let Parts::Fields(fields) = &source.parts else {
        return None;
    };
    if fields.len() != bytes as usize {
        return None;
    }
    // Validate the entire bounded representation before creating any nodes.
    // Storage, repeats, omitted leaves and partially available trees fall back.
    let scalars: Vec<_> = fields
        .iter()
        .map(|field| {
            let Parts::Scalar(node) = &field.parts else {
                return None;
            };
            let node = graph.resolve(*node);
            (field.ty == element && graph.value_type(node) == &ValueType::Rust(element))
                .then_some(node)
        })
        .collect::<Option<_>>()?;
    let mut packed = None;
    for (offset, byte) in scalars.into_iter().enumerate() {
        let mut bits = graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[byte],
            0,
            ValueType::Rust(target),
        );
        if offset != 0 {
            let shift = graph.add_constant((offset * 8) as u128, target);
            bits = graph.add_node(
                Opcode::ShiftLeft,
                NodeId::NONE,
                &[bits, shift],
                0,
                ValueType::Rust(target),
            );
        }
        packed = Some(match packed {
            None => bits,
            Some(previous) => graph.add_node(
                Opcode::BitOr,
                NodeId::NONE,
                &[previous, bits],
                0,
                ValueType::Rust(target),
            ),
        });
    }
    Some(Value {
        ty: target,
        parts: Parts::Scalar(packed?),
    })
}
