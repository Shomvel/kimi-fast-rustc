//! Fuse consecutive bounds tests after publishing L. Both rejected edges must
//! reach the same merge with identical values and effects; no fixed operation
//! or anchored value may sit between the observations.
use crate::analysis::uses::{UseKind, Uses};
use crate::{ComparePredicate as P, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::{IntegerWidth, SizeAlign, TypeOracle, TypeShape};

#[path = "range_union.rs"]
mod union;

#[derive(Clone, Copy)]
enum Bound {
    Lower(u64),
    Upper(u64),
}

struct Comparison {
    value: NodeId,
    bits: u8,
    signed: bool,
    bound: Bound,
}

struct Plan {
    first: NodeId,
    first_pass: usize,
    second: NodeId,
    second_pass: usize,
    value: NodeId,
    signed: bool,
    low: u64,
    span: u64,
    clear: u64,
}

pub(super) fn run(graph: &mut FunctionGraph) -> usize {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static UNION: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if graph.level != GraphLevel::Layout
        || !*ENABLED.get_or_init(|| std::env::var("KFRC_RANGE_TESTS").as_deref() != Ok("0"))
    {
        return 0;
    }
    let unions = *UNION.get_or_init(|| std::env::var("KFRC_RANGE_UNION").as_deref() != Ok("0"));
    let branches: Vec<_> = graph.ids().filter(|&n| graph.op(n) == Opcode::If).collect();
    let mut uses = None;
    let mut plans: Vec<Plan> = Vec::new();
    for first in branches {
        if plans.iter().any(|p| p.first == first || p.second == first) {
            continue;
        }
        // Most branches are not bounds tests. Build one shared index only
        // after seeing an ordered comparison against an integer constant.
        // A comparison at an integer endpoint can have an impossible true
        // side and a valid false side, including through boolean inversions.
        if comparison(graph, first, 0).is_none() && comparison(graph, first, 1).is_none() {
            continue;
        }
        let uses = uses.get_or_insert_with(|| Uses::new(graph));
        if let Some(plan) = plan(graph, uses, first)
            .or_else(|| unions.then(|| union::plan(graph, uses, first)).flatten())
            && !plans
                .iter()
                .any(|p| p.first == plan.second || p.second == plan.second)
        {
            plans.push(plan);
        }
    }
    let mut count = 0;
    for plan in plans {
        let source_ty = graph.rust_type(plan.value).unwrap();
        let TypeShape::Int { width, signed } = *graph.oracle.shape(source_ty) else {
            unreachable!()
        };
        let (mut value, mut ty, mut low) = (plan.value, source_ty, plan.low);
        if width.bits(64) < 32 {
            // A modular narrow SUB needs a destructive byte/word ALU op and
            // usually a copy on x86. Extend to the comparison's signed order
            // first, then measure distance in u32, where negative distances
            // exceed every narrow interval and LEA can compute the offset.
            if signed != plan.signed {
                let narrow = graph.oracle.intern(
                    TypeShape::Int {
                        width,
                        signed: plan.signed,
                    },
                    Some(SizeAlign {
                        size: u64::from(width.bits(64) / 8),
                        align: u64::from(width.bits(64) / 8),
                    }),
                );
                if graph.oracle.scalar_validity(narrow).is_some() {
                    continue;
                }
                value = graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[value],
                    0,
                    ValueType::Rust(narrow),
                );
            }
            ty = graph.oracle.intern(
                TypeShape::Int {
                    width: IntegerWidth::W32,
                    signed: false,
                },
                Some(SizeAlign { size: 4, align: 4 }),
            );
            if graph.oracle.scalar_validity(ty).is_some() {
                continue;
            }
            value = graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[value],
                0,
                ValueType::Rust(ty),
            );
            if plan.signed && low & (1u64 << (width.bits(64) - 1)) != 0 {
                low |= u64::MAX << width.bits(64);
            }
            low &= u64::from(u32::MAX);
        }
        if plan.clear != 0 {
            let bits = match graph.oracle.shape(ty) {
                TypeShape::Int { width, .. } => width.bits(64),
                _ => unreachable!(),
            };
            let mask = (u64::MAX >> (64 - bits)) & !plan.clear;
            let mask = graph.add_constant(u128::from(mask), ty);
            value = graph.add_node(
                Opcode::BitAnd,
                NodeId::NONE,
                &[value, mask],
                0,
                ValueType::Rust(ty),
            );
        }
        let low = graph.add_constant(u128::from(low), ty);
        let span = graph.add_constant(u128::from(plan.span), ty);
        let offset = graph.add_node(
            Opcode::Sub,
            NodeId::NONE,
            &[value, low],
            0,
            ValueType::Rust(ty),
        );
        let boolean = graph.value_type(graph.input(plan.first, 0)).clone();
        // The first branch retains its original passing side.
        let predicate = if plan.first_pass == 0 {
            P::UnsignedLessEqual
        } else {
            P::UnsignedGreater
        };
        let condition = graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[offset, span],
            predicate as u32,
            boolean,
        );
        graph.set_input(plan.first, 0, condition);
        let second_ty = graph.rust_type(graph.input(plan.second, 0)).unwrap();
        let pass = graph.add_constant(u128::from(plan.second_pass == 0), second_ty);
        graph.set_input(plan.second, 0, pass);
        count += 1;
    }
    count
}

fn projections(graph: &FunctionGraph, uses: &Uses, branch: NodeId) -> Option<[NodeId; 2]> {
    let mut edges = [NodeId::NONE; 2];
    if uses.of(branch).len() != 2 {
        return None;
    }
    for usage in uses.of(branch) {
        let node = usage.user;
        if usage.kind != UseKind::Operand(0)
            || graph.op(node) != Opcode::Projection
            || graph.value_type(node) != &ValueType::Control
            || graph.payload(node) > 1
            || !edges[graph.payload(node) as usize].is_none()
        {
            return None;
        }
        edges[graph.payload(node) as usize] = node;
    }
    Some(edges)
}

fn control_user(graph: &FunctionGraph, uses: &Uses, control: NodeId) -> Option<NodeId> {
    let [usage] = uses.of(control) else {
        return None;
    };
    (usage.kind == UseKind::Control && graph.control(usage.user) == control).then_some(usage.user)
}

fn jump_target(graph: &FunctionGraph, uses: &Uses, control: NodeId) -> Option<(NodeId, usize)> {
    let jump = control_user(graph, uses, control)?;
    if graph.op(jump) != Opcode::Jump || !graph.inputs(jump).is_empty() {
        return None;
    }
    let [usage] = uses.of(jump) else { return None };
    let UseKind::Operand(slot) = usage.kind else {
        return None;
    };
    (graph.op(usage.user) == Opcode::Region).then_some((usage.user, slot))
}

fn next_branch(graph: &FunctionGraph, uses: &Uses, mut control: NodeId) -> Option<NodeId> {
    for _ in 0..8 {
        let next = control_user(graph, uses, control)?;
        if graph.op(next) == Opcode::If {
            return Some(next);
        }
        let (region, _) = jump_target(graph, uses, control)?;
        if graph.inputs(region).len() != 1 {
            return None;
        }
        control = region;
    }
    None
}

fn failure(graph: &FunctionGraph, uses: &Uses, mut control: NodeId) -> Option<(NodeId, usize)> {
    for _ in 0..8 {
        let (region, slot) = jump_target(graph, uses, control)?;
        if graph.inputs(region).len() > 1 {
            return Some((region, slot));
        }
        control = region;
    }
    None
}

fn same_merge_values(
    graph: &FunctionGraph,
    uses: &Uses,
    region: NodeId,
    a: usize,
    b: usize,
) -> bool {
    uses.of(region).iter().all(|usage| {
        if usage.kind == UseKind::Control {
            return true;
        }
        let phi = usage.user;
        usage.kind == UseKind::Operand(0)
            && graph.op(phi) == Opcode::Phi
            && graph.inputs(phi).len() == graph.inputs(region).len() + 1
            && graph.resolve(graph.input(phi, a + 1)) == graph.resolve(graph.input(phi, b + 1))
    })
}

fn plan(graph: &FunctionGraph, uses: &Uses, first: NodeId) -> Option<Plan> {
    let first_edges = projections(graph, uses, first)?;
    for first_pass in 0..2 {
        let Some(second) = next_branch(graph, uses, first_edges[first_pass]) else {
            continue;
        };
        let Some(a) = comparison(graph, first, first_pass) else {
            continue;
        };
        let Some((region, first_slot)) = failure(graph, uses, first_edges[1 - first_pass]) else {
            continue;
        };
        let second_edges = projections(graph, uses, second)?;
        for second_pass in 0..2 {
            let Some((other, second_slot)) = failure(graph, uses, second_edges[1 - second_pass])
            else {
                continue;
            };
            if other != region
                || first_slot == second_slot
                || !same_merge_values(graph, uses, region, first_slot, second_slot)
            {
                continue;
            }
            let Some(b) = comparison(graph, second, second_pass) else {
                continue;
            };
            if a.value != b.value || a.bits != b.bits || a.signed != b.signed {
                continue;
            }
            let (low, high) = match (a.bound, b.bound) {
                (Bound::Lower(low), Bound::Upper(high))
                | (Bound::Upper(high), Bound::Lower(low))
                    if low <= high =>
                {
                    (low, high)
                }
                _ => continue,
            };
            return Some(Plan {
                first,
                first_pass,
                second,
                second_pass,
                value: a.value,
                signed: a.signed,
                // Signed intervals are contiguous in sign-biased order. A
                // modular subtraction of the original low bits measures the
                // same distance, including intervals crossing signed zero.
                low: low ^ if a.signed { 1u64 << (a.bits - 1) } else { 0 },
                span: high - low,
                clear: 0,
            });
        }
    }
    None
}

fn comparison(graph: &FunctionGraph, branch: NodeId, pass: usize) -> Option<Comparison> {
    let mut node = graph.resolve(graph.input(branch, 0));
    let mut truth = pass == 0;
    for _ in 0..8 {
        if !graph.control(node).is_none()
            || !graph
                .rust_type(node)
                .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
        {
            return None;
        }
        if graph.op(node) != Opcode::BitNot {
            break;
        }
        truth = !truth;
        node = graph.resolve(graph.input(node, 0));
    }
    if graph.op(node) != Opcode::Compare || !graph.control(node).is_none() {
        return None;
    }
    let (signed, mut lower, mut inclusive) = match P::from_payload(graph.payload(node)) {
        P::SignedLess => (true, false, false),
        P::SignedLessEqual => (true, false, true),
        P::SignedGreater => (true, true, false),
        P::SignedGreaterEqual => (true, true, true),
        P::UnsignedLess => (false, false, false),
        P::UnsignedLessEqual => (false, false, true),
        P::UnsignedGreater => (false, true, false),
        P::UnsignedGreaterEqual => (false, true, true),
        _ => return None,
    };
    let mut value = graph.resolve(graph.input(node, 0));
    let mut constant = graph.resolve(graph.input(node, 1));
    if graph.op(value) == Opcode::Const {
        std::mem::swap(&mut value, &mut constant);
        lower = !lower;
    }
    if !truth {
        lower = !lower;
        inclusive = !inclusive;
    }
    let ty = graph.rust_type(value)?;
    if graph.rust_type(constant) != Some(ty)
        || graph.op(constant) != Opcode::Const
        || graph.oracle.scalar_validity(ty).is_some()
    {
        return None;
    }
    let TypeShape::Int { width, .. } = graph.oracle.shape(ty) else {
        return None;
    };
    let bits = width.bits(64);
    if !matches!(bits, 8 | 16 | 32 | 64) {
        return None;
    }
    let mask = u64::MAX >> (64 - bits);
    let raw = u64::try_from(graph.constant_value(constant)).ok()?;
    if raw > mask {
        return None;
    }
    let ranked = raw ^ if signed { 1u64 << (bits - 1) } else { 0 };
    let bound = match (lower, inclusive) {
        (true, true) => Bound::Lower(ranked),
        (true, false) if ranked < mask => Bound::Lower(ranked + 1),
        (false, true) => Bound::Upper(ranked),
        (false, false) if ranked > 0 => Bound::Upper(ranked - 1),
        _ => return None,
    };
    Some(Comparison {
        value,
        bits,
        signed,
        bound,
    })
}

#[cfg(test)]
#[path = "range_tests_tests.rs"]
mod tests;
