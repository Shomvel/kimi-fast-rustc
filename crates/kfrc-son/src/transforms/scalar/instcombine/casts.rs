//! A cast chain may be replaced only when every result bit has the same
//! source bit (or a proved identical constant). Keep the original source and
//! its poison; do not turn a value proof into a non-poison proof.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bit {
    Source(u8),
    Zero,
}

fn extended(bit: u8, scalar: Integer) -> Bit {
    if bit < scalar.bits {
        Bit::Source(bit)
    } else if scalar.signed {
        Bit::Source(scalar.bits - 1)
    } else {
        Bit::Zero
    }
}

pub(super) fn combine(
    graph: &mut FunctionGraph,
    node: NodeId,
    ty: TyId,
    result: Integer,
) -> Option<NodeId> {
    if graph.op(node) != Opcode::IntCast
        || graph.payload(node) != 0
        || !graph.control(node).is_none()
    {
        return None;
    }
    let middle = graph.resolve(graph.input(node, 0));
    if graph.rust_type(middle) == Some(ty) {
        return Some(middle);
    }
    if graph.op(middle) != Opcode::IntCast
        || graph.payload(middle) != 0
        || !graph.control(middle).is_none()
    {
        return None;
    }
    let intermediate = integer(graph, graph.rust_type(middle)?)?;
    let source = graph.resolve(graph.input(middle, 0));
    let original = integer(graph, graph.rust_type(source)?)?;
    let mut known = None;
    let mut identical = true;
    for bit in 0..result.bits {
        let through = match extended(bit, intermediate) {
            Bit::Source(bit) => extended(bit, original),
            Bit::Zero => Bit::Zero,
        };
        let direct = extended(bit, original);
        if through != direct {
            let facts = match known {
                Some(facts) => facts,
                None => {
                    let Some(facts) = crate::analysis::known_bits::of(graph, source) else {
                        identical = false;
                        break;
                    };
                    known = Some(facts);
                    facts
                }
            };
            let fixed = |bit| match bit {
                Bit::Zero => Some(false),
                Bit::Source(bit) if facts.zero & (1u64 << bit) != 0 => Some(false),
                Bit::Source(bit) if facts.one & (1u64 << bit) != 0 => Some(true),
                _ => None,
            };
            if fixed(through).is_none() || fixed(through) != fixed(direct) {
                identical = false;
                break;
            }
        }
    }
    if !identical {
        // A truncation followed by unsigned extension back to the original
        // Rust type is a mask, not a direct cast. It still observes source
        // poison and replaces two casts with one ordinary operation.
        if intermediate.bits < original.bits
            && !intermediate.signed
            && graph.rust_type(source) == Some(ty)
        {
            let mask = graph.add_constant(u128::from(intermediate.mask()), ty);
            return Some(graph.add_node(
                Opcode::BitAnd,
                NodeId::NONE,
                &[source, mask],
                0,
                graph.value_type(node).clone(),
            ));
        }
        return None;
    }
    Some(if graph.rust_type(source) == Some(ty) {
        source
    } else {
        graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[source],
            0,
            graph.value_type(node).clone(),
        )
    })
}
