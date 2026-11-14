//! Move an ordinary truncation through one bitwise node. Both original
//! operand dependencies remain, including poison from their own contracts.
//! Arithmetic also commutes with truncation, but the current x86 lowering
//! adds canonicalization instructions for those shapes; defer that extension.
use super::{Rule, enabled, record};
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{TyId, TypeOracle, TypeShape};

#[derive(Clone, Copy)]
enum Operand {
    Existing(NodeId),
    Constant { bits: u64, wide: u64 },
    Truncate(NodeId),
}

fn ordinary_bits(graph: &FunctionGraph, ty: TyId) -> Option<u8> {
    if graph.oracle.scalar_validity(ty).is_some() {
        return None;
    }
    match graph.oracle.shape(ty) {
        TypeShape::Int { width, .. } if width.bits(64) <= 64 => Some(width.bits(64)),
        _ => None,
    }
}

fn operand(
    graph: &FunctionGraph,
    node: NodeId,
    wide: TyId,
    narrow: TyId,
    mask: u64,
) -> Option<Operand> {
    let node = graph.resolve(node);
    if graph.rust_type(node) != Some(wide) {
        return None;
    }
    if graph.op(node) == Opcode::Const {
        let wide = graph.constant_value(node) as u64;
        return Some(Operand::Constant {
            bits: wide & mask,
            wide,
        });
    }
    if graph.op(node) == Opcode::IntCast {
        if graph.payload(node) != 0
            || !graph.control(node).is_none()
            || graph.inputs(node).len() != 1
        {
            return None;
        }
        let source = graph.resolve(graph.input(node, 0));
        let source_ty = graph.rust_type(source)?;
        ordinary_bits(graph, source_ty)?;
        if source_ty == narrow {
            // Either sign or zero extension agrees with the source in every
            // low bit. Exact TyId equality also types the new binop operands.
            return Some(Operand::Existing(source));
        }
    }
    Some(Operand::Truncate(node))
}

fn materialize(graph: &mut FunctionGraph, operand: Operand, ty: TyId) -> NodeId {
    match operand {
        Operand::Existing(node) => node,
        Operand::Constant { bits, .. } => graph.add_constant(u128::from(bits), ty),
        Operand::Truncate(node) => graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[node],
            0,
            ValueType::Rust(ty),
        ),
    }
}

/// Batch-only: the caller shares one immutable use snapshot for its sweep.
/// A freshly created producer is left for the next sweep rather than queried
/// in the older snapshot. One-use is a profitability guard;
/// retaining any newly added user still has identical wrapping semantics.
pub(super) fn combine(graph: &mut FunctionGraph, node: NodeId, uses: &Uses) -> Option<NodeId> {
    if !enabled(Rule::TruncBinop)
        || graph.op(node) != Opcode::IntCast
        || graph.payload(node) != 0
        || !graph.control(node).is_none()
        || graph.inputs(node).len() != 1
    {
        return None;
    }
    let narrow = graph.rust_type(node)?;
    let narrow_bits = ordinary_bits(graph, narrow)?;
    let binop = graph.resolve(graph.input(node, 0));
    if !uses.contains(binop)
        || !matches!(
            graph.op(binop),
            Opcode::BitAnd | Opcode::BitOr | Opcode::BitXor
        )
        || graph.payload(binop) != 0
        || !graph.control(binop).is_none()
        || graph.inputs(binop).len() != 2
    {
        return None;
    }
    let wide = graph.rust_type(binop)?;
    let wide_bits = ordinary_bits(graph, wide)?;
    if wide_bits <= narrow_bits {
        return None;
    }
    let mask = u64::MAX >> (64 - narrow_bits);
    let left = operand(graph, graph.input(binop, 0), wide, narrow, mask)?;
    let right = operand(graph, graph.input(binop, 1), wide, narrow, mask)?;
    if matches!(left, Operand::Truncate(_)) || matches!(right, Operand::Truncate(_)) {
        let (immediate, narrowed) = match (left, right) {
            (Operand::Truncate(_), Operand::Constant { wide, bits })
            | (Operand::Constant { wide, bits }, Operand::Truncate(_)) => (wide, bits),
            _ => return None,
        };
        // A new input canonicalization only pays when it removes a 64-bit
        // register immediate. Small immediates already encode directly;
        // moving their mask before a branch increased UTF-8 store work.
        if wide_bits != 64 || i32::try_from(immediate as i64).is_ok() {
            return None;
        }
        // Lowering still uses a 64-bit register operation. An unsigned u32
        // constant with bit 31 set is not a sign-extended imm32 there either;
        // narrowing must actually make the immediate directly encodable.
        if narrow_bits == 32
            && matches!(
                graph.oracle.shape(narrow),
                TypeShape::Int { signed: false, .. }
            )
            && narrowed > i32::MAX as u64
        {
            return None;
        }
    }
    let [usage] = uses.of(binop) else {
        return None;
    };
    if usage.user != node || usage.kind != UseKind::Operand(0) {
        return None;
    }
    let left = materialize(graph, left, narrow);
    let right = materialize(graph, right, narrow);
    let replacement = graph.add_node(
        graph.op(binop),
        NodeId::NONE,
        &[left, right],
        0,
        ValueType::Rust(narrow),
    );
    record(Rule::TruncBinop, graph, node);
    Some(replacement)
}

#[cfg(test)]
#[path = "narrow_tests.rs"]
mod tests;
