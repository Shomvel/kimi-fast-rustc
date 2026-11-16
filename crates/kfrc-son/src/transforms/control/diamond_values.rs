//! A constant arm is redundant when the other expression produces that exact
//! constant under the arm's equality. Build a total wrapping cone for the new
//! use, preserving arithmetic contracts on the original nodes and their other
//! users. A fixed operation or an unrelated value stops the proof.
use super::*;
use crate::ComparePredicate as P;
use crate::transforms::scalar::instcombine;
use kfrc_body::TyId;
use std::cell::Cell;

pub(super) enum Replacement {
    Existing(NodeId),
    Wrapping {
        source: NodeId,
        inputs: Vec<Replacement>,
    },
}

impl Replacement {
    // Plans allocate graph nodes only after every Phi of the diamond passed.
    pub(super) fn materialize(self, graph: &mut FunctionGraph) -> NodeId {
        match self {
            Self::Existing(node) => graph.resolve(node),
            Self::Wrapping { source, inputs } => {
                let inputs: Vec<_> = inputs
                    .into_iter()
                    .map(|input| input.materialize(graph))
                    .collect();
                graph.add_node(
                    graph.op(source),
                    NodeId::NONE,
                    &inputs,
                    0,
                    graph.value_type(source).clone(),
                )
            }
        }
    }
}

pub(super) fn replacement(
    graph: &FunctionGraph,
    condition: NodeId,
    left_truth: bool,
    left: NodeId,
    right: NodeId,
) -> Option<Replacement> {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if graph.level != GraphLevel::Layout
        || !*ENABLED.get_or_init(|| std::env::var("KFRC_DIAMOND_VALUES").as_deref() != Ok("0"))
    {
        return None;
    }
    for (constant, expression, truth) in [(left, right, left_truth), (right, left, !left_truth)] {
        if graph.op(constant) != Opcode::Const || graph.op(expression) == Opcode::Const {
            continue;
        }
        let Some((subject, value)) = equality(graph, condition, truth) else {
            continue;
        };
        let ty = graph.rust_type(expression)?;
        if graph.rust_type(constant) != Some(ty) || graph.rust_type(subject) != Some(ty) {
            continue;
        }
        let mask = ordinary_mask(graph, ty)?;
        let expected = instcombine::constant(graph, constant)?;
        let remaining = Cell::new(32);
        if let Some((actual, replacement)) =
            evaluate(graph, expression, ty, subject, value, mask, &remaining)
        {
            if actual == expected {
                return Some(replacement);
            }
        }
    }
    None
}

fn ordinary_mask(graph: &FunctionGraph, ty: TyId) -> Option<u64> {
    if graph.oracle.scalar_validity(ty).is_some() {
        return None;
    }
    match graph.oracle.shape(ty) {
        TypeShape::Int { width, .. } if width.bits(64) <= 64 => {
            Some(u64::MAX >> (64 - width.bits(64)))
        }
        _ => None,
    }
}

fn equality(
    graph: &FunctionGraph,
    mut condition: NodeId,
    mut truth: bool,
) -> Option<(NodeId, u64)> {
    for _ in 0..8 {
        condition = graph.resolve(condition);
        if !graph.control(condition).is_none() {
            return None;
        }
        if graph.op(condition) != Opcode::BitNot {
            break;
        }
        condition = graph.input(condition, 0);
        truth = !truth;
    }
    if graph.op(condition) != Opcode::Compare {
        return None;
    }
    let a = graph.resolve(graph.input(condition, 0));
    let b = graph.resolve(graph.input(condition, 1));
    let ty = graph.rust_type(a)?;
    let mask = ordinary_mask(graph, ty)?;
    if graph.rust_type(b) != Some(ty) {
        return None;
    }
    let predicate = P::from_payload(graph.payload(condition));
    if matches!((predicate, truth), (P::Equal, true) | (P::NotEqual, false)) {
        for (subject, constant) in [(a, b), (b, a)] {
            if let Some(value) = instcombine::constant(graph, constant) {
                return Some((subject, value));
            }
        }
        return None;
    }
    // A unit wrapping increment overflows only at MAX; a unit decrement
    // underflows only at zero. Normalize the predicate to `less < greater`.
    let (less, greater) = match (predicate, truth) {
        (P::UnsignedLess, true) | (P::UnsignedGreaterEqual, false) => (a, b),
        (P::UnsignedGreater, true) | (P::UnsignedLessEqual, false) => (b, a),
        _ => return None,
    };
    if unit_operation(graph, less, greater, Opcode::Add) {
        Some((greater, mask))
    } else if unit_operation(graph, greater, less, Opcode::Sub) {
        Some((less, 0))
    } else {
        None
    }
}

fn unit_operation(graph: &FunctionGraph, result: NodeId, source: NodeId, op: Opcode) -> bool {
    if graph.op(result) != op
        || graph.payload(result) != 0
        || !graph.control(result).is_none()
        || graph.rust_type(result) != graph.rust_type(source)
    {
        return false;
    }
    let a = graph.resolve(graph.input(result, 0));
    let b = graph.resolve(graph.input(result, 1));
    (a == source && instcombine::constant(graph, b) == Some(1))
        || (op == Opcode::Add && b == source && instcombine::constant(graph, a) == Some(1))
}

fn evaluate(
    graph: &FunctionGraph,
    node: NodeId,
    ty: TyId,
    subject: NodeId,
    value: u64,
    mask: u64,
    remaining: &Cell<usize>,
) -> Option<(u64, Replacement)> {
    let node = graph.resolve(node);
    remaining.set(remaining.get().checked_sub(1)?);
    if graph.rust_type(node) != Some(ty) {
        return None;
    }
    if node == subject {
        return Some((value, Replacement::Existing(node)));
    }
    if !graph.control(node).is_none() {
        return None;
    }
    if graph.op(node) == Opcode::Const {
        return Some((
            instcombine::constant(graph, node)?,
            Replacement::Existing(node),
        ));
    }
    let arity = match graph.op(node) {
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::BitAnd
        | Opcode::BitOr
        | Opcode::BitXor => 2,
        Opcode::BitNot | Opcode::Negate => 1,
        _ => return None,
    };
    if graph.inputs(node).len() != arity {
        return None;
    }
    let mut inputs = Vec::with_capacity(arity);
    let mut constants = [0; 2];
    let mut unchanged = graph.payload(node) == 0;
    for (index, &input) in graph.inputs(node).iter().enumerate() {
        let (constant, replacement) = evaluate(graph, input, ty, subject, value, mask, remaining)?;
        unchanged &=
            matches!(&replacement, Replacement::Existing(old) if *old == graph.resolve(input));
        constants[index] = constant;
        inputs.push(replacement);
    }
    // These operations are total at the declared integer width once their
    // poison contracts are removed. On the original expression arm they
    // preserve every defined result; on the constant arm equality supplies
    // the only variable. Never relax the shared original expression in place.
    let [a, b] = constants;
    let result = match graph.op(node) {
        Opcode::Add => a.wrapping_add(b),
        Opcode::Sub => a.wrapping_sub(b),
        Opcode::Mul => a.wrapping_mul(b),
        Opcode::BitAnd => a & b,
        Opcode::BitOr => a | b,
        Opcode::BitXor => a ^ b,
        Opcode::BitNot => !a,
        Opcode::Negate => a.wrapping_neg(),
        _ => unreachable!(),
    } & mask;
    let replacement = if unchanged {
        Replacement::Existing(node)
    } else {
        Replacement::Wrapping {
            source: node,
            inputs,
        }
    };
    Some((result, replacement))
}

#[cfg(test)]
#[path = "diamond_values_tests.rs"]
mod tests;
