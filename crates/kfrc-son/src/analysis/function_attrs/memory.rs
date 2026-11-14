//! Caller-visible memory effects; private unescaped storage is internal.
use super::*;

pub(super) fn local_effect(
    graph: &FunctionGraph,
    node: NodeId,
    memory: &MemoryFacts,
) -> MemoryEffects {
    let local = |index| {
        graph
            .inputs(node)
            .get(index)
            .and_then(|&address| memory.root(graph.resolve(address)))
            .and_then(|root| memory.local_identity(root))
            .is_some_and(|identity| memory.is_unescaped_local(identity))
    };
    match graph.op(node) {
        Opcode::LoadValue | Opcode::Load => {
            if local(1) {
                MemoryEffects::None
            } else {
                MemoryEffects::ReadOnly
            }
        }
        Opcode::StoreValue
        | Opcode::Store
        | Opcode::SetVariant
        | Opcode::SlotLive
        | Opcode::SlotDead => {
            if local(1) {
                MemoryEffects::None
            } else {
                MemoryEffects::Unknown
            }
        }
        Opcode::CopyNonOverlapping => {
            if !local(2) {
                MemoryEffects::Unknown
            } else if !local(1) {
                MemoryEffects::ReadOnly
            } else {
                MemoryEffects::None
            }
        }
        Opcode::Call
        | Opcode::CallIndirect
        | Opcode::TraitCall
        | Opcode::Opaque
        | Opcode::ThreadLocalAddress
        | Opcode::Hole => MemoryEffects::Unknown,
        Opcode::Start
        | Opcode::Projection
        | Opcode::Region
        | Opcode::Jump
        | Opcode::If
        | Opcode::Switch
        | Opcode::VariantSwitch
        | Opcode::VariantWitness
        | Opcode::Assume
        | Opcode::Phi
        | Opcode::Return
        | Opcode::Unreachable
        | Opcode::Const
        | Opcode::Undefined
        | Opcode::Aggregate
        | Opcode::Repeat
        | Opcode::Field
        | Opcode::InsertField
        | Opcode::MakeVariant
        | Opcode::VariantOf
        | Opcode::VariantField
        | Opcode::WidePtr
        | Opcode::Discriminant
        | Opcode::LocalSlot
        | Opcode::FieldAddr
        | Opcode::VariantFieldAddr
        | Opcode::IndexAddr
        | Opcode::PointerCast
        | Opcode::RequireDefined
        | Opcode::TransmuteValue
        | Opcode::ConstRepr
        | Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::DivSigned
        | Opcode::DivUnsigned
        | Opcode::RemSigned
        | Opcode::RemUnsigned
        | Opcode::BitAnd
        | Opcode::BitOr
        | Opcode::BitXor
        | Opcode::ShiftLeft
        | Opcode::ShiftRightArithmetic
        | Opcode::ShiftRightLogical
        | Opcode::Negate
        | Opcode::BitNot
        | Opcode::Compare
        | Opcode::IntCast
        | Opcode::StackSlot
        | Opcode::SymbolAddress
        | Opcode::PtrOffset
        | Opcode::MulOverflow
        | Opcode::BitCount
        | Opcode::ByteSwap
        | Opcode::BitReverse
        | Opcode::FunnelShift
        | Opcode::SaturatingAdd
        | Opcode::SaturatingSub => MemoryEffects::None,
    }
}
