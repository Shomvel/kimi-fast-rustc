//! Read conditional scalar facts through a verified successful enum field.
//! This never rewrites the caller: splice still observes every original
//! argument, including poison in an inactive field or an unselected sibling.
use super::selected_value;
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{TypeOracle, TypeShape};

pub(super) fn scalar_bits(graph: &FunctionGraph, node: NodeId, fuel: &mut usize) -> Option<u128> {
    *fuel = fuel.checked_sub(1)?;
    let node = selected_value(graph, node, fuel);
    if !graph.control(node).is_none() {
        return None;
    }
    let ty = graph.rust_type(node)?;
    let bits = match graph.oracle.shape(ty) {
        TypeShape::Int { width, .. } if width.bits(64) <= 64 => width.bits(64),
        TypeShape::Bool => 1,
        TypeShape::Char => 32,
        _ => return None,
    };
    let mask = u128::MAX >> (128 - u32::from(bits));
    match graph.op(node) {
        Opcode::Const => Some(graph.constant_value(node) & mask),
        Opcode::Sub => {
            let a = graph.input(node, 0);
            let b = graph.input(node, 1);
            if graph.value_type(a) != graph.value_type(node)
                || graph.value_type(b) != graph.value_type(node)
            {
                return None;
            }
            // Flags may make the original expression poison. The literal
            // describes only its defined executions; its observation remains.
            Some(scalar_bits(graph, a, fuel)?.wrapping_sub(scalar_bits(graph, b, fuel)?) & mask)
        }
        Opcode::VariantField => variant_field(graph, node, fuel),
        _ => None,
    }
}

fn variant_field(graph: &FunctionGraph, node: NodeId, fuel: &mut usize) -> Option<u128> {
    let &[value, witness] = graph.inputs(node) else {
        return None;
    };
    let value = graph.resolve(value);
    let witness = graph.resolve(witness);
    let &(variant, field) = graph.variant_fields.get(graph.payload(node) as usize)?;
    // Driver verification already checks dominance and the dynamic instance.
    // Accept only that exact proof; never infer a witness from a constructor
    // arm or use another enum's successful branch as evidence.
    *fuel = fuel.checked_sub(1)?;
    if graph.op(witness) != Opcode::VariantWitness
        || graph.value_type(witness) != &ValueType::Witness
        || graph.resolve(graph.input(witness, 0)) != value
        || graph.payload(witness) != variant
    {
        return None;
    }
    let enum_ty = graph.rust_type(value)?;
    let semantic = graph.oracle.semantic();
    if !semantic.is_enum(enum_ty)
        || semantic.field_type(enum_ty, Some(variant), field) != graph.rust_type(node)
    {
        return None;
    }
    let arms = match graph.op(value) {
        Opcode::MakeVariant => std::slice::from_ref(&value),
        Opcode::Phi if graph.control(value).is_none() => &graph.inputs(value)[1..],
        _ => return None,
    };
    let mut common = None;
    for &arm in arms {
        *fuel = fuel.checked_sub(1)?;
        let arm = graph.resolve(arm);
        if graph.op(arm) != Opcode::MakeVariant
            || !graph.control(arm).is_none()
            || graph.rust_type(arm) != Some(enum_ty)
        {
            // Unknown and recursive Phi arms cannot establish a fact.
            return None;
        }
        if graph.payload(arm) != variant {
            // This arm does not define the selected field. It contributes no
            // value, and the original witness and argument remain observable.
            continue;
        }
        let payload = *graph.inputs(arm).get(field as usize)?;
        if graph.value_type(payload) != graph.value_type(node) {
            return None;
        }
        let bits = scalar_bits(graph, payload, fuel)?;
        if common.is_some_and(|old| old != bits) {
            return None;
        }
        common = Some(bits);
    }
    common
}
