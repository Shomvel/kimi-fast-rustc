//! Local integer folding and identities on the Rust graph.
//!
//! Rules retain the operand that can carry poison. Memory, calls, control,
//! pointers and trapping arithmetic are outside this transform. The pipeline
//! owns graph compaction and verification state.

use kfrc_body::{TyId, TypeOracle, TypeShape};

use crate::graph::{ComparePredicate, FunctionGraph, NodeId, Opcode};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InstCombineStats {
    pub constants_folded: usize,
    pub identities_simplified: usize,
}

pub(crate) fn run(graph: &mut FunctionGraph) -> InstCombineStats {
    let mut stats = InstCombineStats::default();
    // Forwarding resolves immediately, so one sweep handles straight-line
    // expressions. A few further sweeps catch inputs built after their users
    // without an unbounded fixed-point loop on cyclic SSA graphs.
    for _ in 0..4 {
        let mut changed = false;
        let nodes: Vec<_> = graph.ids().collect();
        for node in nodes {
            if let Some((replacement, constant)) = rewrite(graph, node) {
                graph.replace(node, replacement);
                if constant {
                    stats.constants_folded += 1;
                } else {
                    stats.identities_simplified += 1;
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    stats
}

#[derive(Clone, Copy)]
struct Integer {
    bits: u8,
    signed: bool,
}

/// One scalar rewrite for a seeded worklist. The caller propagates users and
/// keeps NodeIds stable; this does not scan the surrounding graph.
pub(super) fn rewrite(graph: &mut FunctionGraph, node: NodeId) -> Option<(NodeId, bool)> {
    let ty = graph.rust_type(node)?;
    let result = integer(graph, ty)?;
    if let Some(bits) = fold(graph, node) {
        Some((
            graph.add_constant(u128::from(result.normalize(bits)), ty),
            true,
        ))
    } else {
        identity(graph, node, ty, result)
            .or_else(|| combine(graph, node, ty, result))
            .or_else(|| redundant_mask(graph, node, ty, result))
            .map(|replacement| (replacement, false))
    }
}

/// Canonicalize the associative, commutative integer operations. Reassociation
/// retains the possibly-poisoned operand and never duplicates its computation.
/// The shared inner value is left intact for any other consumers.
fn combine(graph: &mut FunctionGraph, node: NodeId, ty: TyId, scalar: Integer) -> Option<NodeId> {
    use Opcode::*;
    let op = graph.op(node);
    if !matches!(op, Add | Mul | BitAnd | BitOr | BitXor)
        || !matches!(
            graph.oracle.shape(ty),
            TypeShape::Int { .. } | TypeShape::Bool
        )
        || graph.payload(node) != 0
        || !graph.control(node).is_none()
    {
        return None;
    }
    let left = graph.resolve(graph.input(node, 0));
    let right = graph.resolve(graph.input(node, 1));
    let a = constant(graph, left);
    let b = constant(graph, right);
    let outer = match (a, b) {
        (None, Some(bits)) => Some((left, bits)),
        (Some(bits), None) => Some((right, bits)),
        _ => None,
    };
    if let Some((inner, outer_bits)) = outer
        && graph.op(inner) == op
        && graph.payload(inner) == 0
        && graph.control(inner).is_none()
        && graph.rust_type(inner) == Some(ty)
    {
        let x = graph.resolve(graph.input(inner, 0));
        let y = graph.resolve(graph.input(inner, 1));
        let pair = match (constant(graph, x), constant(graph, y)) {
            (None, Some(bits)) => Some((x, bits)),
            (Some(bits), None) => Some((y, bits)),
            _ => None,
        };
        if let Some((value, inner_bits)) = pair {
            let bits = scalar.normalize(match op {
                Add => inner_bits.wrapping_add(outer_bits),
                Mul => inner_bits.wrapping_mul(outer_bits),
                BitAnd => inner_bits & outer_bits,
                BitOr => inner_bits | outer_bits,
                BitXor => inner_bits ^ outer_bits,
                _ => unreachable!(),
            });
            let constant = graph.add_constant(u128::from(bits), ty);
            return Some(graph.add_node(
                op,
                NodeId::NONE,
                &[value, constant],
                0,
                graph.value_type(node).clone(),
            ));
        }
    }
    // Constants go on the right for instruction selection. General commutative
    // matching belongs in GVN's key: reordering unrelated values here can
    // perturb tied-register allocation without removing any computation.
    let swap = a.is_some() && b.is_none();
    swap.then(|| {
        graph.add_node(
            op,
            NodeId::NONE,
            &[right, left],
            0,
            graph.value_type(node).clone(),
        )
    })
}

/// A redundant mask may be removed even if its input is poison: keep the
/// original input, including its flags, control dependencies and observation.
fn redundant_mask(
    graph: &FunctionGraph,
    node: NodeId,
    ty: TyId,
    scalar: Integer,
) -> Option<NodeId> {
    if !matches!(graph.op(node), Opcode::BitAnd | Opcode::BitOr)
        || graph.payload(node) != 0
        || !graph.control(node).is_none()
    {
        return None;
    }
    let left = graph.resolve(graph.input(node, 0));
    let right = graph.resolve(graph.input(node, 1));
    let (value, mask) = if let Some(mask) = constant(graph, right) {
        (left, mask)
    } else {
        (right, constant(graph, left)?)
    };
    if graph.rust_type(value) != Some(ty) {
        return None;
    }
    let known = crate::analysis::known_bits::of(graph, value)?;
    let redundant = match graph.op(node) {
        Opcode::BitAnd => (!mask & scalar.mask()) & !known.zero == 0,
        Opcode::BitOr => mask & !known.one == 0,
        _ => unreachable!(),
    };
    redundant.then_some(value)
}

impl Integer {
    fn mask(self) -> u64 {
        u64::MAX >> (64 - self.bits)
    }
    fn normalize(self, value: u64) -> u64 {
        value & self.mask()
    }
    fn signed(self, value: u64) -> i64 {
        let shift = 64 - self.bits;
        ((self.normalize(value) << shift) as i64) >> shift
    }
}

fn integer(graph: &FunctionGraph, ty: TyId) -> Option<Integer> {
    match graph.oracle.shape(ty) {
        TypeShape::Bool => Some(Integer {
            bits: 1,
            signed: false,
        }),
        TypeShape::Char => Some(Integer {
            bits: 32,
            signed: false,
        }),
        TypeShape::Int { width, signed } if width.bits(64) <= 64 => Some(Integer {
            bits: width.bits(64),
            signed: *signed,
        }),
        _ => None,
    }
}

pub(crate) fn constant(graph: &FunctionGraph, node: NodeId) -> Option<u64> {
    let node = graph.resolve(node);
    if graph.op(node) != Opcode::Const {
        return None;
    }
    let scalar = integer(graph, graph.rust_type(node)?)?;
    Some(scalar.normalize(graph.constant_value(node) as u64))
}

fn fold(graph: &FunctionGraph, node: NodeId) -> Option<u64> {
    evaluate(graph, node, |input| constant(graph, input))
}

pub(crate) fn supports_evaluation(op: Opcode) -> bool {
    use Opcode::*;
    op.integer_intrinsic_arity().is_some()
        || matches!(
            op,
            Add | Sub
                | Mul
                | BitAnd
                | BitOr
                | BitXor
                | ShiftLeft
                | ShiftRightArithmetic
                | ShiftRightLogical
                | Negate
                | BitNot
                | IntCast
                | Compare
        )
}

pub(crate) fn evaluate(
    graph: &FunctionGraph,
    node: NodeId,
    constant: impl Fn(NodeId) -> Option<u64>,
) -> Option<u64> {
    let result = integer(graph, graph.rust_type(node)?)?;
    use Opcode::*;
    let op = graph.op(node);
    if op.integer_intrinsic_arity().is_some() {
        let inputs = graph.inputs(node);
        let source = integer(graph, graph.rust_type(inputs[0])?)?;
        let values = inputs
            .iter()
            .map(|&input| constant(input))
            .collect::<Option<Vec<_>>>()?;
        let (value, poison) = crate::intrinsic::evaluate(
            op,
            graph.payload(node),
            source.bits,
            source.signed,
            &values,
        );
        return (!poison).then_some(value);
    }
    if !supports_evaluation(op) {
        return None;
    }
    let inputs = graph.inputs(node);
    let source = integer(graph, graph.rust_type(graph.resolve(*inputs.first()?))?)?;
    let a = constant(inputs[0])?;
    let b = if inputs.len() == 2 {
        constant(inputs[1])?
    } else {
        0
    };
    let shift = (b % u64::from(source.bits)) as u32;
    if crate::ArithmeticFlags::allowed(op).is_some()
        && crate::ArithmeticFlags(graph.payload(node)).violated(op, source.bits, a, b)
    {
        // Retain the operation: replacing poison by a constant would hide the
        // source contract violation from the interpreter and later proofs.
        return None;
    }
    Some(result.normalize(match op {
        Add => a.wrapping_add(b),
        Sub => a.wrapping_sub(b),
        Mul => a.wrapping_mul(b),
        BitAnd => a & b,
        BitOr => a | b,
        BitXor => a ^ b,
        ShiftLeft => a.wrapping_shl(shift),
        ShiftRightLogical => a.wrapping_shr(shift),
        ShiftRightArithmetic => (source.signed(a) >> shift) as u64,
        Negate => a.wrapping_neg(),
        BitNot => !a,
        IntCast => {
            if source.signed {
                source.signed(a) as u64
            } else {
                a
            }
        }
        Compare => u64::from(match ComparePredicate::from_payload(graph.payload(node)) {
            ComparePredicate::Equal => a == b,
            ComparePredicate::NotEqual => a != b,
            ComparePredicate::SignedLess => source.signed(a) < source.signed(b),
            ComparePredicate::SignedLessEqual => source.signed(a) <= source.signed(b),
            ComparePredicate::SignedGreater => source.signed(a) > source.signed(b),
            ComparePredicate::SignedGreaterEqual => source.signed(a) >= source.signed(b),
            ComparePredicate::UnsignedLess => a < b,
            ComparePredicate::UnsignedLessEqual => a <= b,
            ComparePredicate::UnsignedGreater => a > b,
            ComparePredicate::UnsignedGreaterEqual => a >= b,
        }),
        _ => unreachable!(),
    }))
}

fn identity(graph: &FunctionGraph, node: NodeId, ty: TyId, scalar: Integer) -> Option<NodeId> {
    use Opcode::*;
    if graph.op(node) == BitNot {
        let inner = graph.resolve(graph.input(node, 0));
        if graph.op(inner) == BitNot {
            let original = graph.resolve(graph.input(inner, 0));
            return (graph.rust_type(original) == Some(ty)).then_some(original);
        }
    }
    let inputs = graph.inputs(node);
    if matches!(graph.op(node), ByteSwap | BitReverse) {
        let inner = graph.resolve(inputs[0]);
        if graph.op(inner) == graph.op(node) {
            return Some(graph.resolve(graph.input(inner, 0)));
        }
        if graph.op(node) == ByteSwap && scalar.bits == 8 {
            return Some(inner);
        }
    }
    if graph.op(node) == FunnelShift {
        let hi = graph.resolve(inputs[0]);
        let lo = graph.resolve(inputs[1]);
        // An arbitrary unused funnel half may be poison, so only rotate(x,x)
        // may discard one data operand. Do not erase an invalid amount promise.
        if hi == lo
            && let Some(amount) = constant(graph, inputs[2])
        {
            let in_range =
                graph.payload(node) & crate::IntegerIntrinsicPayload::AMOUNT_IN_RANGE != 0;
            if amount % u64::from(scalar.bits) == 0
                && (!in_range || amount < u64::from(scalar.bits))
            {
                return Some(hi);
            }
        }
        return None;
    }
    if inputs.len() != 2 {
        return None;
    }
    let left = graph.resolve(inputs[0]);
    let right = graph.resolve(inputs[1]);
    let right_constant = constant(graph, right);
    let left_constant = constant(graph, left);
    // These identities retain exactly the possibly-poisoned input. Rules such
    // as x*0 -> 0 and x^x -> 0 would incorrectly erase poison in the reference
    // semantics, so they are deliberately absent.
    let replacement = match graph.op(node) {
        Add | BitOr | BitXor if right_constant == Some(0) => left,
        Add | BitOr | BitXor if left_constant == Some(0) => right,
        Sub | ShiftLeft | ShiftRightArithmetic | ShiftRightLogical if right_constant == Some(0) => {
            left
        }
        Mul if right_constant == Some(1) => left,
        Mul if left_constant == Some(1) => right,
        BitAnd if right_constant == Some(scalar.mask()) => left,
        BitAnd if left_constant == Some(scalar.mask()) => right,
        _ => return None,
    };
    (graph.rust_type(replacement) == Some(ty)).then_some(replacement)
}
