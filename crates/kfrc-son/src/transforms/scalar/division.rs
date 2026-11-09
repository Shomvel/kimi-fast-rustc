//! Strength reduction for positive power-of-two integer divisors.
//!
//! Div/Rem are fixed poison observations. Removing them is legal only when
//! the numerator is known defined; propagating poison through a pure shift is
//! insufficient when the result is unused. This first pass intentionally
//! leaves unknown values, zero divisors and signed negative divisors alone.

use kfrc_body::{TyId, TypeOracle, TypeShape};

use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};

#[derive(Clone, Debug, Default)]
pub struct DivisionStats {
    pub divisions_reduced: usize,
    pub remainders_reduced: usize,
    pub nodes_visited: usize,
    pub proof_nodes_visited: usize,
    pub dirty_nodes: Vec<NodeId>,
}

pub fn run(graph: &mut FunctionGraph) -> DivisionStats {
    let mut stats = DivisionStats::default();
    // S entry arguments are checked for poison at the call/entry boundary.
    // R parameters and L ABI lanes do not share that proof contract here.
    if graph.level != GraphLevel::Specialized {
        return stats;
    }
    graph.resolve_edges();
    let mut defined = vec![None; graph.len()];
    let nodes: Vec<_> = graph.ids().collect();
    for node in nodes {
        stats.nodes_visited += 1;
        let opcode = graph.op(node);
        if !matches!(
            opcode,
            Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned
        ) {
            continue;
        }
        let Some(ty) = graph.rust_type(node) else {
            continue;
        };
        let Some((width, signed)) = integer(graph, ty) else {
            continue;
        };
        if signed != matches!(opcode, Opcode::DivSigned | Opcode::RemSigned) {
            continue;
        }
        let numerator = graph.input(node, 0);
        let divisor = graph.input(node, 1);
        if graph.op(divisor) != Opcode::Const {
            continue;
        }
        let mask = u64::MAX >> (64 - width);
        let divisor = graph.constant_value(divisor) as u64 & mask;
        if !divisor.is_power_of_two() || (signed && divisor & (1u64 << (width - 1)) != 0) {
            continue;
        }
        if !known_defined(
            graph,
            numerator,
            &mut defined,
            &mut stats.proof_nodes_visited,
            0,
        ) {
            continue;
        }
        let old_len = graph.len();
        let shift = divisor.trailing_zeros() as u64;
        let remainder = matches!(opcode, Opcode::RemSigned | Opcode::RemUnsigned);
        let value = if !signed {
            if remainder {
                binary_constant(graph, Opcode::BitAnd, numerator, divisor - 1, ty)
            } else if shift == 0 {
                numerator
            } else {
                binary_constant(graph, Opcode::ShiftRightLogical, numerator, shift, ty)
            }
        } else {
            // Negative x needs a bias of d-1 before arithmetic shifting so
            // that division rounds toward zero, not toward negative infinity.
            // For x<0 the biased sum stays in [-2^(W-1), d-2]; for x>=0
            // the bias is zero. Thus the narrow add cannot signed-overflow.
            let quotient = if shift == 0 {
                numerator
            } else {
                let sign = binary_constant(
                    graph,
                    Opcode::ShiftRightArithmetic,
                    numerator,
                    u64::from(width - 1),
                    ty,
                );
                let bias = binary_constant(graph, Opcode::BitAnd, sign, divisor - 1, ty);
                let biased = binary(graph, Opcode::Add, numerator, bias, ty);
                binary_constant(graph, Opcode::ShiftRightArithmetic, biased, shift, ty)
            };
            if remainder {
                let multiple = if shift == 0 {
                    quotient
                } else {
                    binary_constant(graph, Opcode::ShiftLeft, quotient, shift, ty)
                };
                binary(graph, Opcode::Sub, numerator, multiple, ty)
            } else {
                quotient
            }
        };
        graph.replace(node, value);
        if remainder {
            stats.remainders_reduced += 1;
        } else {
            stats.divisions_reduced += 1;
        }
        stats
            .dirty_nodes
            .extend((old_len..graph.len()).map(|n| NodeId(n as u32)));
        stats.dirty_nodes.push(value);
        defined.resize(graph.len(), None);
    }
    graph.resolve_edges();
    stats.dirty_nodes.sort_unstable();
    stats.dirty_nodes.dedup();
    stats
}

fn binary(
    graph: &mut FunctionGraph,
    opcode: Opcode,
    left: NodeId,
    right: NodeId,
    ty: TyId,
) -> NodeId {
    graph.add_node(opcode, NodeId::NONE, &[left, right], 0, ValueType::Rust(ty))
}

fn binary_constant(
    graph: &mut FunctionGraph,
    opcode: Opcode,
    left: NodeId,
    right: u64,
    ty: TyId,
) -> NodeId {
    let right = graph.add_constant(u128::from(right), ty);
    binary(graph, opcode, left, right, ty)
}

fn integer(graph: &FunctionGraph, ty: TyId) -> Option<(u8, bool)> {
    match graph.oracle.shape(ty) {
        TypeShape::Int { width, signed } if width.bits(64) <= 64 => Some((width.bits(64), *signed)),
        _ => None,
    }
}

/// A deliberately small proof, memoized across all candidates. Phi, loads,
/// field extraction and fixed-result projections remain unknown. In
/// particular, a value observed on another branch gives no proof here.
fn known_defined(
    graph: &FunctionGraph,
    node: NodeId,
    memo: &mut [Option<bool>],
    visits: &mut usize,
    depth: usize,
) -> bool {
    let node = graph.resolve(node);
    if depth > 128 {
        return false;
    }
    if let Some(result) = memo[node.index()] {
        return result;
    }
    *visits += 1;
    memo[node.index()] = Some(false);
    if graph
        .rust_type(node)
        .and_then(|ty| integer(graph, ty))
        .is_none()
    {
        return false;
    }
    let result = match graph.op(node) {
        Opcode::Const => true,
        Opcode::Projection => graph.input(node, 0) == graph.start && graph.payload(node) >= 2,
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::BitAnd
        | Opcode::BitOr
        | Opcode::BitXor
        | Opcode::ShiftLeft
        | Opcode::ShiftRightLogical
        | Opcode::ShiftRightArithmetic
        | Opcode::BitNot
        | Opcode::Negate
        | Opcode::IntCast
            if graph.control(node).is_none() =>
        {
            graph
                .inputs(node)
                .iter()
                .all(|&input| known_defined(graph, input, memo, visits, depth + 1))
        }
        _ => false,
    };
    memo[node.index()] = Some(result);
    result
}
