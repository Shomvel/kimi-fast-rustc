//! Bits observed through integer uses, separately from whether a value is
//! poison. Unknown uses demand every bit. Queries merge all uses and stop at
//! cycles, depth eight, or 64 visited use edges; no snapshot survives an edit.
use kfrc_body::{TypeOracle, TypeShape};

use super::uses::{UseKind, Uses};
use crate::{FunctionGraph, NodeId, Opcode};

pub(crate) fn width(graph: &FunctionGraph, node: NodeId) -> Option<(u8, bool)> {
    match graph.oracle.shape(graph.rust_type(graph.resolve(node))?) {
        TypeShape::Bool => Some((1, false)),
        TypeShape::Char => Some((32, false)),
        TypeShape::Int { width, signed } if width.bits(64) <= 64 => Some((width.bits(64), *signed)),
        _ => None,
    }
}

pub(crate) fn mask(bits: u8) -> u64 {
    u64::MAX >> (64 - bits)
}

fn literal(graph: &FunctionGraph, node: NodeId) -> Option<u64> {
    let node = graph.resolve(node);
    (graph.op(node) == Opcode::Const).then(|| graph.constant_value(node) as u64)
}

/// The demand on one operand, given the observed output bits. A flagged or
/// controlled operation is deliberately opaque: its poison contract may
/// observe bits that do not contribute to its ordinary numerical result.
pub(crate) fn operand(
    graph: &FunctionGraph,
    user: NodeId,
    slot: usize,
    demanded: u64,
) -> Option<u64> {
    let user = graph.resolve(user);
    if graph.payload(user) != 0 || !graph.control(user).is_none() {
        return None;
    }
    let (bits, _) = width(graph, user)?;
    let inputs = graph.inputs(user);
    let input = *inputs.get(slot)?;
    let (source_bits, signed) = width(graph, input)?;
    let demanded = demanded & mask(bits);
    let result = match graph.op(user) {
        Opcode::IntCast if slot == 0 => {
            let low = demanded & mask(source_bits);
            if bits > source_bits && signed && demanded & !mask(source_bits) != 0 {
                low | (1u64 << (source_bits - 1))
            } else {
                low
            }
        }
        Opcode::BitAnd | Opcode::BitOr | Opcode::BitXor if slot < 2 => {
            if source_bits != bits {
                return None;
            }
            match (graph.op(user), literal(graph, inputs[1 - slot])) {
                (Opcode::BitAnd, Some(other)) => demanded & other,
                (Opcode::BitOr, Some(other)) => demanded & !other,
                _ => demanded,
            }
        }
        Opcode::BitNot if slot == 0 => demanded,
        Opcode::ShiftLeft | Opcode::ShiftRightLogical | Opcode::ShiftRightArithmetic
            if slot == 0 && source_bits == bits =>
        {
            // KFRC's unpromised shifts mask the amount by the operand width.
            let amount = (literal(graph, inputs[1])? % u64::from(bits)) as u32;
            match graph.op(user) {
                Opcode::ShiftLeft => demanded >> amount,
                Opcode::ShiftRightLogical => demanded << amount,
                Opcode::ShiftRightArithmetic => {
                    let high = mask(bits) ^ (mask(bits) >> amount);
                    (demanded << amount)
                        | if demanded & high != 0 {
                            1u64 << (bits - 1)
                        } else {
                            0
                        }
                }
                _ => unreachable!(),
            }
        }
        // Shift amounts, comparisons, arithmetic, Phi, memory, calls, and
        // aggregate boundaries retain the full input, including observations.
        _ => return None,
    };
    Some(result & mask(source_bits))
}

pub(crate) fn of(graph: &FunctionGraph, uses: &Uses, node: NodeId) -> Option<u64> {
    query(graph, uses, node, &mut Vec::new(), &mut 64)
}

fn query(
    graph: &FunctionGraph,
    uses: &Uses,
    node: NodeId,
    active: &mut Vec<NodeId>,
    remaining: &mut usize,
) -> Option<u64> {
    let node = graph.resolve(node);
    let full = mask(width(graph, node)?.0);
    if active.len() == 8 || active.contains(&node) {
        return Some(full);
    }
    active.push(node);
    let mut demanded = 0;
    for usage in uses.of(node) {
        if *remaining == 0 {
            demanded = full;
            break;
        }
        *remaining -= 1;
        let from_use = match usage.kind() {
            UseKind::Control => full,
            UseKind::Operand(slot) => {
                // Probe the transfer before recursing. Opaque uses must not
                // spend a query traversing unrelated consumers.
                if operand(graph, usage.user, slot, u64::MAX).is_some() {
                    let output = query(graph, uses, usage.user, active, remaining)?;
                    operand(graph, usage.user, slot, output).unwrap_or(full)
                } else {
                    full
                }
            }
        };
        demanded |= from_use;
        if demanded == full {
            break;
        }
    }
    active.pop();
    Some(demanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValueType;
    use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, TyId};

    fn graph() -> (FunctionGraph, TyId, TyId, NodeId) {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: false,
            },
            None,
        );
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            None,
        );
        let mut graph = FunctionGraph::new(
            "demand",
            FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
            types,
        );
        let source = graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(word),
        );
        (graph, word, byte, source)
    }

    fn observe(graph: &mut FunctionGraph, value: NodeId) {
        graph.add_node(
            Opcode::Return,
            NodeId::NONE,
            &[value],
            0,
            ValueType::Control,
        );
    }

    #[test]
    fn the_use_union_covers_disjoint_slices_and_unknown_consumers() {
        let (mut graph, word, byte, source) = graph();
        let low = graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[source],
            0,
            ValueType::Rust(byte),
        );
        observe(&mut graph, low);
        let amount = graph.add_constant(8, word);
        let shift = graph.add_node(
            Opcode::ShiftRightLogical,
            NodeId::NONE,
            &[source, amount],
            0,
            ValueType::Rust(word),
        );
        let high = graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[shift],
            0,
            ValueType::Rust(byte),
        );
        observe(&mut graph, high);
        assert_eq!(of(&graph, &Uses::new(&graph), source), Some(0xffff));
        observe(&mut graph, source);
        assert_eq!(
            of(&graph, &Uses::new(&graph), source),
            Some(u32::MAX.into())
        );
    }

    #[test]
    fn depth_and_edge_budgets_fall_back_to_full_demand() {
        let (mut graph, word, byte, source) = graph();
        let mut last = source;
        for _ in 0..12 {
            last = graph.add_node(
                Opcode::BitNot,
                NodeId::NONE,
                &[last],
                0,
                ValueType::Rust(word),
            );
        }
        let narrow = graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[last],
            0,
            ValueType::Rust(byte),
        );
        observe(&mut graph, narrow);
        assert_eq!(
            of(&graph, &Uses::new(&graph), source),
            Some(u32::MAX.into())
        );

        let (mut graph, _, byte, source) = self::graph();
        for _ in 0..80 {
            let narrow = graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[source],
                0,
                ValueType::Rust(byte),
            );
            observe(&mut graph, narrow);
        }
        assert_eq!(
            of(&graph, &Uses::new(&graph), source),
            Some(u32::MAX.into())
        );
    }

    #[test]
    fn a_cycle_does_not_claim_that_no_bits_are_observed() {
        let (mut graph, word, _, _) = graph();
        let first = graph.add_node(Opcode::BitNot, NodeId::NONE, &[], 0, ValueType::Rust(word));
        let second = graph.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[first],
            0,
            ValueType::Rust(word),
        );
        graph.set_inputs(first, &[second]);
        assert_eq!(of(&graph, &Uses::new(&graph), first), Some(u32::MAX.into()));
    }
}
