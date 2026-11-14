//! Exact small-step trip proof for wrapping integer recurrences. No general
//! SCEV, nowrap or progress facts are inferred from this bounded simulation.
use super::Work;
use crate::analysis::induction::{Induction as AddRec, constant as literal};
use crate::{ComparePredicate, FunctionGraph, NodeId, Opcode};
use kfrc_body::{TyId, TypeOracle, TypeShape};

pub(super) struct Induction {
    pub phi: NodeId,
    pub ty: TyId,
    /// Header values, including the final failing test. Body count is len - 1.
    pub values: Vec<u64>,
}

fn comparison(
    graph: &FunctionGraph,
    branch: NodeId,
    body: NodeId,
    work: &mut Work,
) -> Option<(NodeId, bool)> {
    let mut condition = graph.resolve(graph.input(branch, 0));
    let mut keep_when = graph.payload(body) == 0;
    // Rust's lowered while condition commonly branches on !comparison. Follow
    // only floating Bool negations: integer complement and fixed observations
    // do not have this branch-polarity rule. Charging each hop also bounds a
    // malformed cycle without changing any graph nodes.
    loop {
        work.charge(1)?;
        if !graph.control(condition).is_none() {
            return None;
        }
        if graph.op(condition) == Opcode::Compare {
            return Some((condition, keep_when));
        }
        if graph.op(condition) != Opcode::BitNot
            || !graph
                .rust_type(condition)
                .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
        {
            return None;
        }
        keep_when = !keep_when;
        condition = graph.resolve(graph.input(condition, 0));
    }
}

pub(super) fn prove(
    graph: &FunctionGraph,
    branch: NodeId,
    body: NodeId,
    phis: &[super::plan::Phi],
    entry_slot: usize,
    limit: usize,
    work: &mut Work,
) -> Option<Induction> {
    let (condition, keep_when) = comparison(graph, branch, body, work)?;
    let left = graph.resolve(graph.input(condition, 0));
    let right = graph.resolve(graph.input(condition, 1));
    let (phi, bound, swapped) = if let Some(phi) = phis.iter().find(|phi| phi.node == left) {
        (phi, literal(graph, right)?, false)
    } else {
        (
            phis.iter().find(|phi| phi.node == right)?,
            literal(graph, left)?,
            true,
        )
    };
    let ty = graph.rust_type(phi.node)?;
    let TypeShape::Int { width, .. } = graph.oracle.shape(ty) else {
        return None;
    };
    let bits = width.bits(64);
    if bits > 64 {
        return None;
    }
    let mask = u64::MAX >> (64 - bits);
    let signed_value = |value: u64| {
        let shift = 64 - bits;
        i128::from((((value & mask) << shift) as i64) >> shift)
    };
    let recurrence = AddRec::analyze(graph, phi.node, entry_slot)?;
    if recurrence.step == 0 {
        return None;
    }
    let predicate = ComparePredicate::from_payload(graph.payload(condition));
    let mut current = literal(graph, recurrence.initial)? & mask;
    let bound = bound & mask;
    let mut values = vec![current];
    for iteration in 0..=limit {
        work.charge(1)?;
        let (a, b) = if swapped {
            (bound, current)
        } else {
            (current, bound)
        };
        let result = match predicate {
            ComparePredicate::Equal => a == b,
            ComparePredicate::NotEqual => a != b,
            ComparePredicate::SignedLess => signed_value(a) < signed_value(b),
            ComparePredicate::SignedLessEqual => signed_value(a) <= signed_value(b),
            ComparePredicate::SignedGreater => signed_value(a) > signed_value(b),
            ComparePredicate::SignedGreaterEqual => signed_value(a) >= signed_value(b),
            ComparePredicate::UnsignedLess => a < b,
            ComparePredicate::UnsignedLessEqual => a <= b,
            ComparePredicate::UnsignedGreater => a > b,
            ComparePredicate::UnsignedGreaterEqual => a >= b,
        };
        if result != keep_when {
            return Some(Induction {
                phi: phi.node,
                ty,
                values,
            });
        }
        if iteration == limit {
            work.limits_hit += 1;
            return None;
        }
        // A payload-zero Add/Sub has exact modular semantics. Simulate every
        // executed update at its own width, including the one before exit.
        // This can prove a short wrapping loop without claiming no-wrap or
        // termination for any other start, bound, step or number of iterations.
        current = current.wrapping_add(recurrence.step) & mask;
        values.push(current);
    }
    None
}
