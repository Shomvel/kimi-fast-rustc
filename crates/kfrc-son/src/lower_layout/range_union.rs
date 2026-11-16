//! Union of two intervals differing only in one input bit. Clearing that bit
//! maps exactly those two intervals to one, including on rejecting inputs.
use super::*;

struct Interval {
    value: NodeId,
    low: u64,
    span: u64,
}

fn interval(graph: &FunctionGraph, branch: NodeId, pass: usize) -> Option<Interval> {
    let mut condition = graph.resolve(graph.input(branch, 0));
    let mut truth = pass == 0;
    for _ in 0..8 {
        if !graph.control(condition).is_none() {
            return None;
        }
        if graph.op(condition) != Opcode::BitNot {
            break;
        }
        condition = graph.resolve(graph.input(condition, 0));
        truth = !truth;
    }
    if graph.op(condition) != Opcode::Compare {
        return None;
    }
    match (P::from_payload(graph.payload(condition)), truth) {
        (P::UnsignedLessEqual, true) | (P::UnsignedGreater, false) => (),
        _ => return None,
    }
    let offset = graph.resolve(graph.input(condition, 0));
    let span = graph.resolve(graph.input(condition, 1));
    if graph.op(offset) != Opcode::Sub
        || graph.payload(offset) != 0
        || !graph.control(offset).is_none()
        || graph.op(span) != Opcode::Const
    {
        return None;
    }
    let value = graph.resolve(graph.input(offset, 0));
    let low = graph.resolve(graph.input(offset, 1));
    let ty = graph.rust_type(value)?;
    if graph.op(low) != Opcode::Const
        || graph.rust_type(span) != Some(ty)
        || graph.rust_type(low) != Some(ty)
        || graph.oracle.scalar_validity(ty).is_some()
    {
        return None;
    }
    let TypeShape::Int {
        width,
        signed: false,
    } = graph.oracle.shape(ty)
    else {
        return None;
    };
    let bits = width.bits(64);
    if !matches!(bits, 8 | 16 | 32 | 64) {
        return None;
    }
    let max = u64::MAX >> (64 - bits);
    let low = u64::try_from(graph.constant_value(low)).ok()?;
    let span = u64::try_from(graph.constant_value(span)).ok()?;
    (low.checked_add(span)? <= max).then_some(Interval { value, low, span })
}

pub(super) fn plan(graph: &FunctionGraph, uses: &Uses, first: NodeId) -> Option<Plan> {
    let first_edges = projections(graph, uses, first)?;
    for first_pass in 0..2 {
        let Some(a) = interval(graph, first, first_pass) else {
            continue;
        };
        let Some(second) = next_branch(graph, uses, first_edges[1 - first_pass]) else {
            continue;
        };
        let Some((region, a_slot)) = failure(graph, uses, first_edges[first_pass]) else {
            continue;
        };
        let second_edges = projections(graph, uses, second)?;
        for second_pass in 0..2 {
            let Some(b) = interval(graph, second, second_pass) else {
                continue;
            };
            let Some((other, b_slot)) = failure(graph, uses, second_edges[second_pass]) else {
                continue;
            };
            if !same_value(graph, a.value, b.value, 0)
                || a.span != b.span
                || other != region
                || a_slot == b_slot
                || !same_merge_values(graph, uses, region, a_slot, b_slot)
            {
                continue;
            }
            let low = a.low.min(b.low);
            let high = a.low.max(b.low);
            let delta = high - low;
            // Both endpoints share the same bit-clear half of one period.
            // Merely testing endpoint bits would miss intervals spanning a
            // whole period, so require the span to fit below that bit as well.
            if !delta.is_power_of_two()
                || a.span >= delta
                || low & delta != 0
                || low.checked_add(a.span)? & delta != 0
                || high != (low | delta)
            {
                continue;
            }
            return Some(Plan {
                first,
                first_pass,
                second,
                second_pass: 1 - second_pass,
                value: a.value,
                signed: false,
                low,
                span: a.span,
                clear: delta,
            });
        }
    }
    None
}

// Separately fused ranges may each have added the same extension. Compare
// only this bounded cast prefix; loads, arithmetic and Phis remain SSA values.
fn same_value(graph: &FunctionGraph, a: NodeId, b: NodeId, depth: usize) -> bool {
    a == b
        || (depth < 2
            && graph.op(a) == Opcode::IntCast
            && graph.op(b) == Opcode::IntCast
            && graph.payload(a) == 0
            && graph.payload(b) == 0
            && graph.control(a).is_none()
            && graph.control(b).is_none()
            && graph.value_type(a) == graph.value_type(b)
            && same_value(
                graph,
                graph.resolve(graph.input(a, 0)),
                graph.resolve(graph.input(b, 0)),
                depth + 1,
            ))
}
