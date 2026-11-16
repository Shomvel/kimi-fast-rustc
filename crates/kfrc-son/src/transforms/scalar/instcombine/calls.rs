//! Equality-only runtime comparisons. Authentication carries compare_bytes'
//! full-range read/initialization contract; a same-named user call has none.
use super::{Rule, constant, enabled, record};
use crate::analysis::uses::{UseKind, Uses};
use crate::{ComparePredicate, FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{IntegerWidth, RuntimeComparison, TyId, TypeOracle, TypeShape};

fn result_type(graph: &FunctionGraph, call: NodeId) -> Option<TyId> {
    if graph.op(call) != Opcode::Call
        || graph.inputs(call).len() != 4
        || graph.runtime_comparison(graph.payload(call)) != Some(RuntimeComparison::Memcmp)
    {
        return None;
    }
    // Only expose the existing exact-length x86 equality expansion. Dynamic
    // and larger comparisons still call libc; on the measured glibc the two
    // names resolve to the same implementation and only perturb link layout.
    let length = graph.resolve(graph.input(call, 3));
    if graph.op(length) != Opcode::Const || graph.constant_value(length) > 32 {
        return None;
    }
    let ValueType::Tuple(fields) = graph.value_type(call) else {
        return None;
    };
    let [ValueType::Control, ValueType::Effect, ValueType::Rust(ty)] = fields.as_slice() else {
        return None;
    };
    if graph.oracle.scalar_validity(*ty).is_some()
        || !matches!(
            graph.oracle.shape(*ty),
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: true,
            }
        )
    {
        return None;
    }
    for slot in [1, 2] {
        let ty = graph.rust_type(graph.resolve(graph.input(call, slot)))?;
        if !matches!(graph.oracle.shape(ty), TypeShape::RawPointer { .. }) {
            return None;
        }
    }
    let length = graph.rust_type(graph.resolve(graph.input(call, 3)))?;
    if graph.oracle.scalar_validity(length).is_some()
        || !matches!(graph.oracle.shape(length), TypeShape::Int { width, signed: false }
            if width.bits(64) == 64)
    {
        return None;
    }
    Some(*ty)
}

/// The replacement can change any nonzero result. Every path out of the
/// return-value/Phi closure must therefore observe only whether it is zero.
fn equality_uses(graph: &FunctionGraph, call: NodeId, ty: TyId, uses: &Uses) -> bool {
    const MAX_VALUES: usize = 128;
    const MAX_EDGES: usize = 512;
    let mut roots = Vec::new();
    for usage in uses.of(call) {
        let user = usage.user;
        if usage.kind() != UseKind::Operand(0)
            || graph.op(user) != Opcode::Projection
            || graph.inputs(user).len() != 1
            || !graph.control(user).is_none()
        {
            return false;
        }
        match graph.payload(user) {
            0 | 1 => (),
            2 if graph.rust_type(user) == Some(ty) => roots.push(user),
            _ => return false,
        }
    }
    if roots.is_empty() {
        return false;
    }
    // 1 = on the DFS stack, 2 = fully checked. A back edge rejects cyclic
    // value propagation instead of treating a visited Phi as a proof.
    let mut colors = std::collections::BTreeMap::<NodeId, u8>::new();
    let mut stack: Vec<_> = roots.into_iter().map(|node| (node, false)).collect();
    let mut edges = 0;
    let mut comparisons = 0;
    while let Some((value, exiting)) = stack.pop() {
        if exiting {
            colors.insert(value, 2);
            continue;
        }
        match colors.get(&value) {
            Some(1) => return false,
            Some(2) => continue,
            _ => (),
        }
        if colors.len() == MAX_VALUES || !uses.contains(value) || uses.of(value).is_empty() {
            return false;
        }
        colors.insert(value, 1);
        stack.push((value, true));
        for usage in uses.of(value) {
            edges += 1;
            if edges > MAX_EDGES {
                return false;
            }
            let user = usage.user;
            let UseKind::Operand(slot) = usage.kind() else {
                return false;
            };
            match graph.op(user) {
                Opcode::Phi
                    if slot > 0
                        && graph.rust_type(user) == Some(ty)
                        && graph.payload(user) == 0
                        && graph.control(user).is_none() =>
                {
                    stack.push((user, false));
                }
                Opcode::Compare
                    if slot < 2
                        && graph.inputs(user).len() == 2
                        && graph.control(user).is_none()
                        && matches!(
                            ComparePredicate::from_payload(graph.payload(user)),
                            ComparePredicate::Equal | ComparePredicate::NotEqual
                        ) =>
                {
                    let other = graph.resolve(graph.input(user, 1 - slot));
                    if graph.rust_type(other) != Some(ty) || constant(graph, other) != Some(0) {
                        return false;
                    }
                    comparisons += 1;
                }
                _ => return false,
            }
        }
    }
    comparisons != 0
}

pub(super) fn run(graph: &mut FunctionGraph) -> usize {
    if !enabled(Rule::MemcmpZero) {
        return 0;
    }
    let candidates: Vec<_> = graph
        .ids()
        .filter_map(|call| result_type(graph, call).map(|ty| (call, ty)))
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    let uses = Uses::new(graph);
    let mut changed = 0;
    for (call, ty) in candidates {
        if equality_uses(graph, call, ty, &uses) {
            let callee = graph.add_runtime_comparison(RuntimeComparison::Bcmp);
            // Preserve the original call's argument observations, projections
            // and fixed control/effect edges, even when length is zero.
            graph.set_payload(call, callee);
            record(Rule::MemcmpZero, graph, call);
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;
