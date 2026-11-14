//! Small strength reductions that retain every possibly-poisoned input.
use super::{Integer, Rule, constant, enabled, integer, record};
use crate::analysis::uses::{UseKind, Uses};
use crate::graph::{FunctionGraph, NodeId, Opcode};
use kfrc_body::{TyId, TypeOracle, TypeShape};

fn plain(graph: &FunctionGraph, node: NodeId, ty: TyId) -> bool {
    graph.rust_type(node) == Some(ty) && graph.payload(node) == 0 && graph.control(node).is_none()
}

fn ordinary_integer(graph: &FunctionGraph, ty: TyId) -> Option<Integer> {
    if graph.oracle.scalar_validity(ty).is_some()
        || !matches!(graph.oracle.shape(ty), TypeShape::Int { .. })
    {
        return None;
    }
    integer(graph, ty)
}

fn shift_amount(graph: &FunctionGraph, node: NodeId) -> Option<NodeId> {
    let node = graph.resolve(node);
    ordinary_integer(graph, graph.rust_type(node)?)?;
    Some(node)
}

fn one_use(uses: &Uses, node: NodeId, user: NodeId, slot: usize) -> bool {
    if !uses.contains(node) {
        return false;
    }
    matches!(uses.of(node), [usage] if usage.user == user && usage.kind == UseKind::Operand(slot))
}

fn current_use(graph: &FunctionGraph, value: NodeId, usage: &crate::analysis::uses::Use) -> bool {
    if graph.is_dead(usage.user) {
        return false;
    }
    match usage.kind {
        UseKind::Operand(slot) => graph
            .inputs(usage.user)
            .get(slot)
            .is_some_and(|&input| graph.resolve(input) == value),
        UseKind::Control => graph.resolve(graph.control(usage.user)) == value,
    }
}

fn observed_result(graph: &FunctionGraph, uses: &Uses, value: NodeId) -> bool {
    let mut pending = vec![value];
    let mut visited = Vec::new();
    let mut edges = 0;
    while let Some(value) = pending.pop() {
        if visited.contains(&value) {
            continue;
        }
        if visited.len() == 32 || !uses.contains(value) {
            // A cost proof may conservatively retain the original multiply.
            return true;
        }
        visited.push(value);
        for usage in uses.of(value) {
            edges += 1;
            if edges > 128 {
                return true;
            }
            if current_use(graph, value, usage) {
                if graph.op(usage.user).is_fixed() || !graph.control(usage.user).is_none() {
                    return true;
                }
                pending.push(usage.user);
            }
        }
    }
    false
}

/// Preserve a potentially free low product from the unsigned 64-bit overflow
/// check. This is the integer domain of x86 lower/checked.rs::key; signed and
/// narrow products cannot share that backend result. Equal constants may be
/// separate nodes here and merge during subsequent GVN.
fn shares_checked_product(
    graph: &FunctionGraph,
    uses: &Uses,
    source: NodeId,
    value: NodeId,
    factor: u64,
    ty: TyId,
    scalar: Integer,
) -> bool {
    if scalar.signed || scalar.bits != 64 {
        return false;
    }
    // A preceding rewrite can forward this operand after the snapshot. Query
    // both its old and current identities, validating every edge against the
    // current graph. A wholly new source waits for the next batch snapshot.
    if !uses.contains(source) && !uses.contains(value) {
        return true;
    }
    let mut edges = 0;
    for identity in [source, value] {
        if !uses.contains(identity) {
            continue;
        }
        for usage in uses.of(identity) {
            edges += 1;
            if edges > 128 {
                return true;
            }
            let overflow = usage.user;
            if !matches!(usage.kind, UseKind::Operand(0 | 1))
                || !current_use(graph, value, usage)
                || graph.op(overflow) != Opcode::MulOverflow
                || graph.payload(overflow) != 0
                || graph.inputs(overflow).len() != 2
                || !uses.contains(overflow)
            {
                continue;
            }
            for (value_slot, factor_slot) in [(0, 1), (1, 0)] {
                let shared = graph.resolve(graph.input(overflow, value_slot));
                let other = graph.resolve(graph.input(overflow, factor_slot));
                if shared == value
                    && graph.rust_type(shared) == Some(ty)
                    && graph.rust_type(other) == Some(ty)
                    && constant(graph, other) == Some(factor)
                    && observed_result(graph, uses, overflow)
                {
                    return true;
                }
            }
        }
        if source == value {
            break;
        }
    }
    false
}

fn mul_power_of_two(
    graph: &mut FunctionGraph,
    node: NodeId,
    ty: TyId,
    scalar: Integer,
    uses: &Uses,
) -> Option<NodeId> {
    if !enabled(Rule::MulPowerOfTwo) {
        return None;
    }
    for (value, factor) in [(0, 1), (1, 0)] {
        let source = graph.input(node, value);
        let value = graph.resolve(source);
        let factor = graph.resolve(graph.input(node, factor));
        if graph.rust_type(value) != Some(ty) || graph.rust_type(factor) != Some(ty) {
            continue;
        }
        let Some(bits) = constant(graph, factor) else {
            continue;
        };
        // Multiplication by one is already an identity. Zero must still
        // consume the possibly-poisoned value and is not a power of two.
        if bits <= 1 || !bits.is_power_of_two() {
            continue;
        }
        let amount = bits.trailing_zeros();
        if amount >= u32::from(scalar.bits) {
            continue;
        }
        if shares_checked_product(graph, uses, source, value, bits, ty, scalar) {
            return None;
        }
        let amount = graph.add_constant(u128::from(amount), ty);
        let replacement = graph.add_node(
            Opcode::ShiftLeft,
            NodeId::NONE,
            &[value, amount],
            0,
            graph.value_type(node).clone(),
        );
        record(Rule::MulPowerOfTwo, graph, node);
        return Some(replacement);
    }
    None
}

fn opposite_shifts(
    graph: &mut FunctionGraph,
    node: NodeId,
    ty: TyId,
    scalar: Integer,
    uses: &Uses,
) -> Option<NodeId> {
    if !enabled(Rule::OppositeShifts) {
        return None;
    }
    let inner = graph.resolve(graph.input(node, 0));
    let expected = match graph.op(node) {
        Opcode::ShiftLeft => Opcode::ShiftRightLogical,
        Opcode::ShiftRightLogical => Opcode::ShiftLeft,
        _ => return None,
    };
    if graph.op(inner) != expected || !plain(graph, inner, ty) {
        return None;
    }
    let amount =
        constant(graph, shift_amount(graph, graph.input(node, 1))?)? % u64::from(scalar.bits);
    let inner_amount =
        constant(graph, shift_amount(graph, graph.input(inner, 1))?)? % u64::from(scalar.bits);
    // The snapshot can predate a forwarded input created earlier in this
    // sweep. Leave it for a later sweep instead of indexing beyond Uses.
    if amount != inner_amount || !one_use(uses, inner, node, 0) {
        return None;
    }
    let value = graph.resolve(graph.input(inner, 0));
    if graph.rust_type(value) != Some(ty) {
        return None;
    }
    let mask = if graph.op(node) == Opcode::ShiftRightLogical {
        scalar.mask() >> amount
    } else {
        (scalar.mask() << amount) & scalar.mask()
    };
    let mask = graph.add_constant(u128::from(mask), ty);
    let replacement = graph.add_node(
        Opcode::BitAnd,
        NodeId::NONE,
        &[value, mask],
        0,
        graph.value_type(node).clone(),
    );
    record(Rule::OppositeShifts, graph, node);
    Some(replacement)
}

fn mul_shift_factor(
    graph: &mut FunctionGraph,
    node: NodeId,
    ty: TyId,
    uses: &Uses,
) -> Option<NodeId> {
    if !enabled(Rule::MulShiftFactor) {
        return None;
    }
    for (value_slot, factor_slot) in [(0, 1), (1, 0)] {
        let value = graph.resolve(graph.input(node, value_slot));
        let factor = graph.resolve(graph.input(node, factor_slot));
        if graph.rust_type(value) != Some(ty)
            || graph.op(factor) != Opcode::ShiftLeft
            || !plain(graph, factor, ty)
            || constant(graph, graph.input(factor, 0)) != Some(1)
            || !one_use(uses, factor, node, factor_slot)
        {
            continue;
        }
        let Some(amount) = shift_amount(graph, graph.input(factor, 1)) else {
            continue;
        };
        // Reuse the amount itself. Both old and new shifts apply the same
        // operand-width mask, including amounts at or above the bit width.
        let replacement = graph.add_node(
            Opcode::ShiftLeft,
            NodeId::NONE,
            &[value, amount],
            0,
            graph.value_type(node).clone(),
        );
        record(Rule::MulShiftFactor, graph, node);
        return Some(replacement);
    }
    None
}

pub(super) fn combine(graph: &mut FunctionGraph, node: NodeId, uses: &Uses) -> Option<NodeId> {
    let ty = graph.rust_type(node)?;
    if !plain(graph, node, ty) {
        return None;
    }
    let scalar = ordinary_integer(graph, ty)?;
    match graph.op(node) {
        Opcode::Mul => mul_power_of_two(graph, node, ty, scalar, uses)
            .or_else(|| mul_shift_factor(graph, node, ty, uses)),
        Opcode::ShiftLeft | Opcode::ShiftRightLogical => {
            opposite_shifts(graph, node, ty, scalar, uses)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "strength_tests.rs"]
mod tests;
