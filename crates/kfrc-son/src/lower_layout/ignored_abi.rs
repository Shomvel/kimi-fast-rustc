//! Ignore describes transport only; source values survive until publishing L.
use super::*;
use kfrc_body::{PassMode, TagEncoding};

pub(super) fn erase_ignored_transport(graph: &mut FunctionGraph) -> Result<(), LayoutError> {
    let unit = graph
        .oracle
        .intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let placeholder = graph.add_constant(0, unit);
    let nodes: Vec<_> = graph.ids().collect();
    // Remove only transport edges. Effects, source reads and calls stay rooted.
    for &node in &nodes {
        match graph.op(node) {
            Opcode::Return if graph.abi.ret == PassMode::Ignore => {
                graph.set_inputs(node, &[graph.input(node, 0)]);
            }
            Opcode::Call | Opcode::CallIndirect => {
                let abi = if graph.op(node) == Opcode::Call {
                    graph.callees[graph.payload(node) as usize].abi.clone()
                } else {
                    graph.call_abis[graph.payload(node) as usize].clone()
                };
                let first = if graph.op(node) == Opcode::Call { 1 } else { 2 };
                for (index, mode) in abi.args.iter().enumerate() {
                    if *mode == PassMode::Ignore {
                        graph.set_input(node, first + index, placeholder);
                    }
                }
            }
            _ => {}
        }
    }
    graph.compact();
    let mut projections = projections::ProjectionIndex::new(graph);
    let nodes: Vec<_> = graph.ids().collect();
    for node in nodes {
        let ignored: Vec<u32> = match graph.op(node) {
            Opcode::Start => graph
                .abi
                .args
                .iter()
                .enumerate()
                .filter_map(|(i, mode)| (*mode == PassMode::Ignore).then_some(2 + i as u32))
                .collect(),
            Opcode::Call
                if graph.callees[graph.payload(node) as usize].abi.ret == PassMode::Ignore =>
            {
                vec![2]
            }
            Opcode::CallIndirect
                if graph.call_abis[graph.payload(node) as usize].ret == PassMode::Ignore =>
            {
                vec![2]
            }
            _ => continue,
        };
        for index in ignored {
            if let Some(result) = projections.find(graph, node, index) {
                let ty = graph.rust_type(result).ok_or_else(|| {
                    LayoutError::Malformed("ignored result lacks source type".into())
                })?;
                let value = zst_value(graph, ty)?;
                graph.replace(result, value);
            }
            let ValueType::Tuple(mut fields) = graph.value_type(node).clone() else {
                continue;
            };
            if let Some(field) = fields.get_mut(index as usize) {
                *field = ValueType::Rust(unit);
            }
            graph.set_value_type(node, ValueType::Tuple(fields));
        }
    }
    graph.compact();
    Ok(())
}

fn zst_value(graph: &mut FunctionGraph, ty: TyId) -> Result<NodeId, LayoutError> {
    if !graph
        .oracle
        .size_align(ty)
        .is_some_and(|layout| layout.size == 0)
    {
        return Err(LayoutError::Malformed(
            "used Ignore value is not zero-sized".into(),
        ));
    }
    if matches!(graph.oracle.shape(ty), TypeShape::Never) {
        return Err(LayoutError::Unsupported(
            "cannot reconstruct an inhabited value of Never".into(),
        ));
    }
    if graph.oracle.semantic().maybe_uninit_inner(ty).is_some() {
        // The empty raw initialization mask is valid even for MaybeUninit<!>.
        return Ok(graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Rust(ty)));
    }
    if graph.oracle.semantic().is_enum(ty) {
        let variant = match graph.oracle.semantic().inhabited_variants(ty) {
            Some([variant]) => *variant,
            Some(_) => {
                return Err(LayoutError::Unsupported(
                    "ignored enum is not uniquely inhabited".into(),
                ));
            }
            None => match graph.oracle.enums.get(&ty).map(|layout| &layout.encoding) {
                Some(TagEncoding::Single { variant, .. }) => *variant,
                _ => {
                    return Err(LayoutError::Unsupported(
                        "ignored enum lacks a unique inhabited variant".into(),
                    ));
                }
            },
        };
        let types = graph
            .oracle
            .semantic()
            .field_types(ty, Some(variant))
            .ok_or_else(|| LayoutError::Malformed("ignored enum lacks fields".into()))?;
        let fields = types
            .into_iter()
            .map(|field| zst_value(graph, field))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &fields,
            variant,
            ValueType::Rust(ty),
        ));
    }
    if matches!(
        graph.oracle.shape(ty),
        TypeShape::Unit | TypeShape::FunctionItem { .. }
    ) {
        return Ok(graph.add_constant(0, ty));
    }
    // A zero-length array is inhabited even when its element type is Never.
    if let TypeShape::Array { element, length } = graph.oracle.shape(ty) {
        let (element, length) = (*element, *length);
        if length == 0 {
            return Ok(graph.add_node(
                Opcode::Aggregate,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(ty),
            ));
        }
        let element = zst_value(graph, element)?;
        return Ok(graph.add_node(
            Opcode::Repeat,
            NodeId::NONE,
            &[element],
            0,
            ValueType::Rust(ty),
        ));
    }
    let fields = graph
        .oracle
        .semantic()
        .field_types(ty, None)
        .ok_or_else(|| {
            LayoutError::Unsupported("ignored value lacks a reconstructible ZST shape".into())
        })?;
    let fields = fields
        .into_iter()
        .map(|field| zst_value(graph, field))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &fields,
        0,
        ValueType::Rust(ty),
    ))
}
