//! Integer comparison implication after the caller validates proof scope.
//! Values remain SSA identities: this does not equate separate loads or fields.

use kfrc_body::TypeShape;

use crate::{ComparePredicate as P, FunctionGraph, NodeId, Opcode};

#[derive(Clone, Copy)]
struct Comparison {
    left: NodeId,
    right: NodeId,
    predicate: P,
}

impl Comparison {
    fn read(graph: &FunctionGraph, node: NodeId) -> Option<Self> {
        (graph.op(node) == Opcode::Compare).then(|| Self {
            left: graph.resolve(graph.input(node, 0)),
            right: graph.resolve(graph.input(node, 1)),
            predicate: P::from_payload(graph.payload(node)),
        })
    }

    fn swapped(self) -> Self {
        Self {
            left: self.right,
            right: self.left,
            predicate: match self.predicate {
                P::SignedLess => P::SignedGreater,
                P::SignedLessEqual => P::SignedGreaterEqual,
                P::SignedGreater => P::SignedLess,
                P::SignedGreaterEqual => P::SignedLessEqual,
                P::UnsignedLess => P::UnsignedGreater,
                P::UnsignedLessEqual => P::UnsignedGreaterEqual,
                P::UnsignedGreater => P::UnsignedLess,
                P::UnsignedGreaterEqual => P::UnsignedLessEqual,
                predicate => predicate,
            },
        }
    }
}

fn signed(predicate: P) -> Option<bool> {
    match predicate {
        P::Equal | P::NotEqual => None,
        P::SignedLess | P::SignedLessEqual | P::SignedGreater | P::SignedGreaterEqual => Some(true),
        _ => Some(false),
    }
}

// Ordered outcomes: less, equal, greater. Negating a predicate complements
// this set; swapping operands exchanges less and greater.
fn outcomes(predicate: P) -> u8 {
    match predicate {
        P::Equal => 2,
        P::NotEqual => 5,
        P::SignedLess | P::UnsignedLess => 1,
        P::SignedLessEqual | P::UnsignedLessEqual => 3,
        P::SignedGreater | P::UnsignedGreater => 4,
        P::SignedGreaterEqual | P::UnsignedGreaterEqual => 6,
    }
}

/// Known and queried conditions have already had Boolean negation stripped.
/// No answer is produced without a previously observed comparison.
pub(super) fn compare(
    graph: &FunctionGraph,
    known: NodeId,
    truth: bool,
    query: NodeId,
) -> Option<bool> {
    let known = Comparison::read(graph, known)?;
    let query = Comparison::read(graph, query)?;
    let ty = graph.rust_type(known.left)?;
    if [known.right, query.left, query.right]
        .into_iter()
        .any(|value| graph.rust_type(value) != Some(ty))
    {
        return None;
    }
    let semantic = graph.oracle.semantic();
    let TypeShape::Int { width, .. } = semantic.shape(ty) else {
        return None;
    };
    if let (Some(a), Some(b)) = (signed(known.predicate), signed(query.predicate)) {
        if a != b {
            return None;
        }
    }
    // Pointer-sized integers use the only supported target's 64-bit width.
    let bits = width.bits(64);
    let is_signed = signed(known.predicate)
        .or_else(|| signed(query.predicate))
        .unwrap_or(false);
    let max = u128::MAX >> (128 - bits);
    let sign = if is_signed { 1u128 << (bits - 1) } else { 0 };
    for known in [known, known.swapped()] {
        for query in [query, query.swapped()] {
            if known.left != query.left {
                continue;
            }
            let known_outcomes = outcomes(known.predicate) ^ if truth { 0 } else { 7 };
            let query_outcomes = outcomes(query.predicate);
            if known.right == query.right {
                if known_outcomes & !query_outcomes == 0 {
                    return Some(true);
                }
                if known_outcomes & query_outcomes == 0 {
                    return Some(false);
                }
                // Matching operands can still have stronger endpoint facts:
                // x != 0 implies !(x <u 0), even though != permits either
                // ordering in the abstract three-outcome relation.
            }
            // A varying RHS gives an existential bound on the common operand.
            // The queried RHS must be constant; no relation between two
            // different unknown operands is invented.
            if graph.op(query.right) != Opcode::Const {
                continue;
            }
            let rhs = if graph.op(known.right) == Opcode::Const {
                let value = (graph.constant_value(known.right) & max) ^ sign;
                (value, value)
            } else {
                (0, max)
            };
            let possible = allowed(known_outcomes, rhs, max);
            let boundary = (graph.constant_value(query.right) & max) ^ sign;
            let required = allowed(query_outcomes, (boundary, boundary), max);
            if possible.iter().all(Option::is_none) {
                continue;
            }
            if possible
                .iter()
                .flatten()
                .all(|&(lo, hi)| required.iter().flatten().any(|&(a, b)| a <= lo && hi <= b))
            {
                return Some(true);
            }
            if possible
                .iter()
                .flatten()
                .all(|&(lo, hi)| required.iter().flatten().all(|&(a, b)| hi < a || b < lo))
            {
                return Some(false);
            }
        }
    }
    None
}

/// Closed intervals in unsigned order (signed values are biased by the sign
/// bit). Bounds avoid MAX+1, including for 128-bit integers. At most two
/// intervals are needed, for inequality against an exact constant.
fn allowed(relation: u8, (lo, hi): (u128, u128), max: u128) -> [Option<(u128, u128)>; 2] {
    match relation {
        1 => [hi.checked_sub(1).map(|end| (0, end)), None],
        2 => [Some((lo, hi)), None],
        3 => [Some((0, hi)), None],
        4 => [
            lo.checked_add(1)
                .filter(|&start| start <= max)
                .map(|start| (start, max)),
            None,
        ],
        5 if lo != hi => [Some((0, max)), None],
        5 => [
            lo.checked_sub(1).map(|end| (0, end)),
            hi.checked_add(1)
                .filter(|&start| start <= max)
                .map(|start| (start, max)),
        ],
        6 => [Some((lo, max)), None],
        _ => unreachable!("integer comparison outcome set"),
    }
}

#[cfg(test)]
#[path = "implication_tests.rs"]
mod tests;
