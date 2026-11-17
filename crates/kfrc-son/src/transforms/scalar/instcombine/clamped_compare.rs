//! Compare a range-clamped unsigned integer without constructing its selection.
//! Both outcomes still consume the original source, including its poison.
use super::{Integer, constant, integer};
use crate::{ComparePredicate as P, FunctionGraph, GraphLevel, NodeId, Opcode};
use kfrc_body::{TyId, TypeOracle, TypeShape};

fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("KFRC_CLAMP_COMPARE").as_deref() != Ok("0"))
}

fn pure(graph: &FunctionGraph, node: NodeId, op: Opcode, ty: TyId) -> bool {
    graph.op(node) == op
        && graph.payload(node) == 0
        && graph.control(node).is_none()
        && graph.rust_type(node) == Some(ty)
}

fn ordinary(graph: &FunctionGraph, ty: TyId) -> Option<Integer> {
    if !matches!(graph.oracle.shape(ty), TypeShape::Int { .. })
        || graph.oracle.scalar_validity(ty).is_some()
    {
        return None;
    }
    integer(graph, ty)
}

// Same-width signedness casts preserve the bit equality of an ordinary integer.
fn equality_bits(graph: &FunctionGraph, mut node: NodeId) -> Option<NodeId> {
    for _ in 0..2 {
        let ty = graph.rust_type(node)?;
        if !pure(graph, node, Opcode::IntCast, ty) {
            break;
        }
        let target = ordinary(graph, ty)?;
        let source = graph.resolve(graph.input(node, 0));
        if ordinary(graph, graph.rust_type(source)?)?.bits != target.bits {
            break;
        }
        node = source;
    }
    Some(node)
}

struct Clamp {
    source: NodeId,
    ty: TyId,
    limit: u64,
    fallback: u64,
}

fn selection(graph: &FunctionGraph, node: NodeId) -> Option<Clamp> {
    let ty = graph.rust_type(node)?;
    if !pure(graph, node, Opcode::BitOr, ty) {
        return None;
    }
    let word = ordinary(graph, ty)?;
    if word.signed {
        return None;
    }
    for (yes_slot, no_slot) in [(0, 1), (1, 0)] {
        let yes = graph.resolve(graph.input(node, yes_slot));
        let no = graph.resolve(graph.input(node, no_slot));
        if !pure(graph, yes, Opcode::BitAnd, ty) || !pure(graph, no, Opcode::BitAnd, ty) {
            continue;
        }
        for (mask_slot, value_slot) in [(0, 1), (1, 0)] {
            let mask = graph.resolve(graph.input(yes, mask_slot));
            let value = graph.resolve(graph.input(yes, value_slot));
            if !pure(graph, mask, Opcode::Negate, ty) {
                continue;
            }
            let cast = graph.resolve(graph.input(mask, 0));
            if !pure(graph, cast, Opcode::IntCast, ty) {
                continue;
            }
            let predicate = graph.resolve(graph.input(cast, 0));
            if graph.op(predicate) != Opcode::Compare
                || graph.payload(predicate) != P::UnsignedLessEqual as u32
                || graph.control(predicate) != NodeId::NONE
                || !graph
                    .rust_type(predicate)
                    .is_some_and(|t| matches!(graph.oracle.shape(t), TypeShape::Bool))
            {
                continue;
            }
            let source = graph.resolve(graph.input(predicate, 0));
            let source_ty = graph.rust_type(source)?;
            let scalar = ordinary(graph, source_ty)?;
            if scalar.signed || scalar.bits > word.bits {
                continue;
            }
            let limit_node = graph.resolve(graph.input(predicate, 1));
            if graph.rust_type(limit_node) != Some(source_ty) {
                continue;
            }
            let Some(limit) = constant(graph, limit_node) else {
                continue;
            };
            let source_value = if pure(graph, value, Opcode::IntCast, ty) {
                graph.resolve(graph.input(value, 0))
            } else if source_ty == ty {
                value
            } else {
                continue;
            };
            if source_value != source {
                continue;
            }
            for (inverse_slot, fallback_slot) in [(0, 1), (1, 0)] {
                let inverse = graph.resolve(graph.input(no, inverse_slot));
                let fallback = graph.resolve(graph.input(no, fallback_slot));
                if !pure(graph, inverse, Opcode::BitNot, ty)
                    || graph.resolve(graph.input(inverse, 0)) != mask
                    || graph.rust_type(fallback) != Some(ty)
                {
                    continue;
                }
                let Some(fallback) = constant(graph, fallback) else {
                    continue;
                };
                if fallback > limit {
                    return Some(Clamp {
                        source,
                        ty: source_ty,
                        limit,
                        fallback,
                    });
                }
            }
        }
    }
    None
}

// For C>K, the fallback cannot equal any E<=K. Equality with such E
// therefore implies x<=K and reduces to x==E. Equality with C instead
// selects exactly x>K. Keep every other expected value unchanged, and keep
// the original selection available to its remaining users.
pub(super) fn combine(graph: &mut FunctionGraph, node: NodeId) -> Option<NodeId> {
    if graph.level != GraphLevel::Layout
        || graph.op(node) != Opcode::Compare
        || graph.control(node) != NodeId::NONE
        || !enabled()
    {
        return None;
    }
    let predicate = P::from_payload(graph.payload(node));
    if !matches!(predicate, P::Equal | P::NotEqual) {
        return None;
    }
    let (mut value, mut expected) = (
        graph.resolve(graph.input(node, 0)),
        graph.resolve(graph.input(node, 1)),
    );
    if graph.op(value) == Opcode::Const {
        std::mem::swap(&mut value, &mut expected);
    }
    if graph.rust_type(expected) != graph.rust_type(value) {
        return None;
    }
    let expected = constant(graph, expected)?;
    let selected = selection(graph, equality_bits(graph, value)?)?;
    let (op, bits) = if expected <= selected.limit {
        (predicate, expected)
    } else if expected == selected.fallback {
        (
            if predicate == P::Equal {
                P::UnsignedGreater
            } else {
                P::UnsignedLessEqual
            },
            selected.limit,
        )
    } else {
        return None;
    };
    let constant = graph.add_constant(u128::from(bits), selected.ty);
    Some(graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[selected.source, constant],
        op as u32,
        graph.value_type(node).clone(),
    ))
}

#[cfg(test)]
#[path = "clamped_compare_tests.rs"]
mod tests;
