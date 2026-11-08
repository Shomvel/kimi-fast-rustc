//! SoN verifier (`docs/design/ir/verifier.md`), P0 subset.
//!
//! Structural: inputs exist, arities, `Phi`/`Region` correspondence,
//! projections. Typed: operand and result types per opcode. Full: control
//! reachability and dominance — every value's earliest placement must
//! dominate each use (a `Phi` operand: the matching predecessor edge). The
//! dominance check is what rejects the T01 miscompile shape, where a value
//! defined in one branch is used after the merge.

use std::collections::HashMap;

use kfrc_body::{PassMode, TypeOracle, TypeShape};

use crate::graph::{FunctionGraph, GraphState, NodeId, Opcode, ValueType};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    DanglingInput { node: NodeId, input: usize },
    WrongArity { node: NodeId, expected: String, actual: usize },
    MissingControl { node: NodeId },
    PhiArity { phi: NodeId, region: NodeId },
    BadRegionInput { region: NodeId, input: NodeId },
    BadProjection { node: NodeId },
    TypeMismatch { node: NodeId, detail: String },
    Unreachable { node: NodeId },
    DominanceViolation { definition: NodeId, user: NodeId },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "graph verification failed: {self:?}")
    }
}

impl std::error::Error for VerifyError {}

/// Runs all checks; on success marks the graph `Verified`.
pub fn verify_graph(graph: &mut FunctionGraph) -> Result<(), VerifyError> {
    check(graph)?;
    graph.state = GraphState::Verified;
    Ok(())
}

pub fn check(graph: &FunctionGraph) -> Result<(), VerifyError> {
    structural(graph)?;
    typed(graph)?;
    dominance(graph)
}

fn arity(node: NodeId, expected: &str, actual: usize, ok: bool) -> Result<(), VerifyError> {
    if ok {
        Ok(())
    } else {
        Err(VerifyError::WrongArity { node, expected: expected.to_owned(), actual })
    }
}

fn structural(graph: &FunctionGraph) -> Result<(), VerifyError> {
    let count = graph.len();
    for node in graph.ids() {
        let inputs = graph.inputs(node);
        for (index, input) in inputs.iter().enumerate() {
            if input.is_none() || input.index() >= count || graph.is_dead(*input) {
                return Err(VerifyError::DanglingInput { node, input: index });
            }
        }
        let control = graph.control(node);
        if !control.is_none() && (control.index() >= count || graph.is_dead(control)) {
            return Err(VerifyError::MissingControl { node });
        }
        let op = graph.op(node);
        let needs_control = matches!(
            op,
            Opcode::Jump
                | Opcode::If
                | Opcode::Return
                | Opcode::Unreachable
                | Opcode::Call
                | Opcode::DivSigned
                | Opcode::DivUnsigned
                | Opcode::RemSigned
                | Opcode::RemUnsigned
        );
        if needs_control && control.is_none() {
            return Err(VerifyError::MissingControl { node });
        }
        let n = inputs.len();
        match op {
            Opcode::Start | Opcode::Const | Opcode::Undefined | Opcode::Jump | Opcode::Unreachable => {
                arity(node, "0", n, n == 0)?
            }
            Opcode::Projection => {
                arity(node, "1", n, n == 1)?;
                let source = inputs[0];
                let limit = match graph.op(source) {
                    Opcode::Start => 2 + graph.abi.args.len() as u32,
                    Opcode::If => 2,
                    Opcode::Call => 3,
                    _ => 0,
                };
                if graph.payload(node) >= limit {
                    return Err(VerifyError::BadProjection { node });
                }
            }
            Opcode::Region => {
                arity(node, ">= 1", n, n >= 1)?;
                for input in inputs {
                    if graph.op(*input) != Opcode::Jump {
                        return Err(VerifyError::BadRegionInput { region: node, input: *input });
                    }
                }
            }
            Opcode::Phi => {
                arity(node, ">= 2", n, n >= 2)?;
                let region = inputs[0];
                if graph.op(region) != Opcode::Region || graph.inputs(region).len() != n - 1 {
                    return Err(VerifyError::PhiArity { phi: node, region });
                }
            }
            Opcode::If | Opcode::Negate | Opcode::BitNot | Opcode::IntCast => arity(node, "1", n, n == 1)?,
            Opcode::Return => {
                let expected = if graph.abi.ret == PassMode::Ignore { 1 } else { 2 };
                arity(node, &expected.to_string(), n, n == expected)?
            }
            Opcode::Call => {
                let callee = &graph.callees[graph.payload(node) as usize];
                let expected = 1 + callee.abi.args.len();
                arity(node, &expected.to_string(), n, n == expected)?
            }
            Opcode::Compare => arity(node, "2", n, n == 2)?,
            op if op.is_binary_arithmetic() => arity(node, "2", n, n == 2)?,
            _ => {}
        }
    }
    Ok(())
}

fn rust_type(graph: &FunctionGraph, node: NodeId) -> Option<kfrc_body::TyId> {
    graph.rust_type(node)
}

fn mismatch(node: NodeId, detail: impl Into<String>) -> VerifyError {
    VerifyError::TypeMismatch { node, detail: detail.into() }
}

fn typed(graph: &FunctionGraph) -> Result<(), VerifyError> {
    let is_integer_like = |node: NodeId| {
        rust_type(graph, node).is_some_and(|ty| {
            matches!(graph.oracle.shape(ty), TypeShape::Int { .. } | TypeShape::Bool | TypeShape::Char)
        })
    };
    for node in graph.ids() {
        let inputs = graph.inputs(node);
        match graph.op(node) {
            op if op.is_binary_arithmetic() => {
                let shift = matches!(op, Opcode::ShiftLeft | Opcode::ShiftRightArithmetic | Opcode::ShiftRightLogical);
                let result = rust_type(graph, node);
                if rust_type(graph, inputs[0]) != result || (!shift && rust_type(graph, inputs[1]) != result) {
                    return Err(mismatch(node, "binary operands must have the result type"));
                }
                if !is_integer_like(inputs[0]) || !is_integer_like(inputs[1]) {
                    return Err(mismatch(node, "binary operands must be integers or bool"));
                }
            }
            Opcode::Negate | Opcode::BitNot => {
                if rust_type(graph, inputs[0]) != rust_type(graph, node) || !is_integer_like(inputs[0]) {
                    return Err(mismatch(node, "unary operand must have the result type"));
                }
            }
            Opcode::IntCast => {
                if !is_integer_like(inputs[0]) || !is_integer_like(node) {
                    return Err(mismatch(node, "IntToInt casts integers"));
                }
            }
            Opcode::Compare => {
                if rust_type(graph, inputs[0]) != rust_type(graph, inputs[1]) {
                    return Err(mismatch(node, "compared values must have the same type"));
                }
                if !rust_type(graph, node).is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool)) {
                    return Err(mismatch(node, "comparison result must be bool"));
                }
            }
            Opcode::If => {
                if !rust_type(graph, inputs[0]).is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool)) {
                    return Err(mismatch(node, "branch condition must be bool"));
                }
            }
            Opcode::Phi => {
                let ty = graph.value_type(node);
                for operand in &inputs[1..] {
                    if graph.value_type(*operand) != ty {
                        return Err(mismatch(node, format!("phi operand {operand} has a different type")));
                    }
                }
            }
            Opcode::Return | Opcode::Call => {
                if *graph.value_type(inputs[0]) != ValueType::Effect {
                    return Err(mismatch(node, "first input must be the effect token"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Dominance over the control skeleton, with value placement as early as
/// the inputs allow.
fn dominance(graph: &FunctionGraph) -> Result<(), VerifyError> {
    // Control-flow nodes and their successors.
    let mut successors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let is_control_flow = |node: NodeId| {
        let op = graph.op(node);
        op.is_fixed() || (op == Opcode::Projection && matches!(graph.value_type(node), ValueType::Control))
    };
    for node in graph.ids() {
        if !is_control_flow(node) {
            continue;
        }
        let control = graph.control(node);
        if !control.is_none() {
            successors.entry(control).or_default().push(node);
        }
        match graph.op(node) {
            Opcode::Projection => successors.entry(graph.input(node, 0)).or_default().push(node),
            Opcode::Region => {
                for jump in graph.inputs(node) {
                    successors.entry(*jump).or_default().push(node);
                }
            }
            _ => {}
        }
    }
    // Reverse postorder from Start.
    let start = graph.start;
    let mut order = Vec::new();
    let mut visited: HashMap<NodeId, bool> = HashMap::new();
    let mut stack = vec![(start, 0usize)];
    visited.insert(start, true);
    while let Some((node, next)) = stack.pop() {
        let list = successors.get(&node).cloned().unwrap_or_default();
        if next < list.len() {
            stack.push((node, next + 1));
            let successor = list[next];
            if let std::collections::hash_map::Entry::Vacant(entry) = visited.entry(successor) {
                entry.insert(true);
                stack.push((successor, 0));
            }
        } else {
            order.push(node);
        }
    }
    order.reverse();
    let position: HashMap<NodeId, usize> = order.iter().enumerate().map(|(index, node)| (*node, index)).collect();
    for node in graph.ids() {
        if is_control_flow(node) && !position.contains_key(&node) {
            return Err(VerifyError::Unreachable { node });
        }
    }
    // Predecessors and immediate dominators (Cooper, Harvey, Kennedy).
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); order.len()];
    for (index, node) in order.iter().enumerate() {
        for successor in successors.get(node).into_iter().flatten() {
            predecessors[position[successor]].push(index);
        }
    }
    let mut idom: Vec<usize> = vec![usize::MAX; order.len()];
    idom[0] = 0;
    let mut changed = true;
    while changed {
        changed = false;
        for index in 1..order.len() {
            let mut new_idom = usize::MAX;
            for &predecessor in &predecessors[index] {
                if idom[predecessor] == usize::MAX {
                    continue;
                }
                new_idom = if new_idom == usize::MAX {
                    predecessor
                } else {
                    intersect(&idom, predecessor, new_idom)
                };
            }
            if new_idom != idom[index] {
                idom[index] = new_idom;
                changed = true;
            }
        }
    }
    let dominates = |a: usize, mut b: usize| loop {
        if a == b {
            return true;
        }
        if b == 0 {
            return false;
        }
        b = idom[b];
    };
    // Earliest placement of every node, as a position in the control order.
    let mut placement: HashMap<NodeId, usize> = HashMap::new();
    let ids: Vec<NodeId> = graph.ids().collect();
    let place = |node: NodeId, placement: &mut HashMap<NodeId, usize>| -> Result<usize, VerifyError> {
        place_node(graph, node, &position, &dominates, placement)
    };
    for node in &ids {
        place(*node, &mut placement)?;
    }
    // Uses by fixed nodes and phis.
    for node in ids {
        let op = graph.op(node);
        if op == Opcode::Phi {
            let region = graph.input(node, 0);
            let jumps = graph.inputs(region);
            for (slot, operand) in graph.inputs(node)[1..].iter().enumerate() {
                let definition = placement[operand];
                if !dominates(definition, position[&jumps[slot]]) {
                    return Err(VerifyError::DominanceViolation { definition: *operand, user: node });
                }
            }
        } else if is_control_flow(node) && op != Opcode::Projection && op != Opcode::Region {
            let at = position[&node];
            for input in graph.inputs(node) {
                if !dominates(placement[input], at) {
                    return Err(VerifyError::DominanceViolation { definition: *input, user: node });
                }
            }
        }
    }
    Ok(())
}

fn intersect(idom: &[usize], mut a: usize, mut b: usize) -> usize {
    while a != b {
        while a > b {
            a = idom[a];
        }
        while b > a {
            b = idom[b];
        }
    }
    a
}

fn place_node(
    graph: &FunctionGraph,
    node: NodeId,
    position: &HashMap<NodeId, usize>,
    dominates: &dyn Fn(usize, usize) -> bool,
    placement: &mut HashMap<NodeId, usize>,
) -> Result<usize, VerifyError> {
    if let Some(at) = placement.get(&node) {
        return Ok(*at);
    }
    let op = graph.op(node);
    let at = if let Some(at) = position.get(&node) {
        *at
    } else {
        match op {
            Opcode::Const | Opcode::Undefined => 0,
            Opcode::Phi => position[&graph.input(node, 0)],
            Opcode::Projection => {
                // A value result of Start or Call sits where its source sits.
                let source = graph.input(node, 0);
                place_node(graph, source, position, dominates, placement)?
            }
            _ => {
                // Pure: the deepest input placement; inputs must lie on one
                // dominator chain.
                let mut deepest = 0usize;
                let mut deepest_input = NodeId::NONE;
                for input in graph.inputs(node).to_vec() {
                    let input_at = place_node(graph, input, position, dominates, placement)?;
                    if dominates(deepest, input_at) {
                        deepest = input_at;
                        deepest_input = input;
                    } else if !dominates(input_at, deepest) {
                        return Err(VerifyError::DominanceViolation {
                            definition: if deepest_input.is_none() { input } else { deepest_input },
                            user: node,
                        });
                    }
                }
                deepest
            }
        }
    };
    placement.insert(node, at);
    Ok(at)
}
