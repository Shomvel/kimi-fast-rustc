//! Expose a single small memcpy consumer of initialized private integer lanes.
//! Values are snapshotted at the call's incoming effect before any byte write;
//! the ordinary private-load pass keeps its original closed-use contract.
use super::*;
use kfrc_body::{Mutability, PassMode};

#[path = "private_copy_chunks.rs"]
mod chunks;

struct Copy {
    node: NodeId,
    destination: NodeId,
    source: NodeId,
    bytes: u64,
    byte: TyId,
    count_ty: TyId,
}

struct Plan {
    copy: Copy,
    values: Vec<Forward>,
}

fn integer(graph: &FunctionGraph, ty: TyId) -> Option<u64> {
    let TypeShape::Int { width, .. } = graph.oracle.shape(ty) else {
        return None;
    };
    let bytes = u64::from(width.bits(64)) / 8;
    if !matches!(bytes, 1 | 2 | 4 | 8)
        || graph.oracle.size_align(ty)?.size != bytes
        || graph
            .oracle
            .scalar_validity(ty)
            .is_some_and(|validity| validity.lanes().any(|(_, range)| !range.is_full()))
    {
        return None;
    }
    Some(bytes)
}

fn byte_pointer(graph: &FunctionGraph, node: NodeId, mutable: bool) -> Option<TyId> {
    let ValueType::Rust(ty) = *graph.value_type(node) else {
        return None;
    };
    let TypeShape::RawPointer {
        mutability,
        pointee,
    } = *graph.oracle.shape(ty)
    else {
        return None;
    };
    if (mutable && mutability != Mutability::Mut)
        || graph.oracle.size_align(ty)?.size != 8
        || !matches!(
            graph.oracle.shape(pointee),
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            }
        )
        || integer(graph, pointee)? != 1
    {
        return None;
    }
    Some(pointee)
}

fn descriptor(graph: &FunctionGraph, node: NodeId, uses: &Uses) -> Option<Copy> {
    if graph.op(node) != Opcode::Call || graph.inputs(node).len() != 4 {
        return None;
    }
    // This is the existing L memcpy libcall contract from semantic_memory,
    // not a licence to reinterpret an arbitrary same-named call signature.
    let callee = graph.callees.get(graph.payload(node) as usize)?;
    if callee.symbol != "memcpy"
        || callee.abi.args != [PassMode::Direct; 3]
        || callee.abi.ret != PassMode::Direct
        || callee.abi.c_variadic
        || graph.value_type(graph.input(node, 0)) != &ValueType::Effect
        || graph.control(node).is_none()
    {
        return None;
    }
    let destination = graph.input(node, 1);
    let source = graph.input(node, 2);
    let byte = byte_pointer(graph, destination, true)?;
    byte_pointer(graph, source, false)?;
    let ValueType::Tuple(results) = graph.value_type(node) else {
        return None;
    };
    if results.len() != 3
        || results[..2] != [ValueType::Control, ValueType::Effect]
        || results[2] != *graph.value_type(destination)
        || uses.of(node).iter().any(|u| {
            u.kind != UseKind::Operand(0)
                || graph.op(u.user) != Opcode::Projection
                || graph.payload(u.user) > 2
        })
    {
        return None;
    }
    let count = graph.input(node, 3);
    let count_ty = graph.rust_type(count)?;
    if graph.op(count) != Opcode::Const
        || !matches!(
            graph.oracle.shape(count_ty),
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            }
        )
        || integer(graph, count_ty)? != 8
    {
        return None;
    }
    let bytes = u64::try_from(graph.constant_value(count)).ok()?;
    (1..=8).contains(&bytes).then_some(Copy {
        node,
        destination,
        source,
        bytes,
        byte,
        count_ty,
    })
}

fn allocation(graph: &FunctionGraph, mut address: NodeId) -> Option<NodeId> {
    // Constant nonnegative offsets only; collect_with_copy checks the full
    // derived-address use graph and the exact allocation bounds afterwards.
    for _ in 0..64 {
        match graph.op(address) {
            Opcode::StackSlot => return Some(address),
            Opcode::PtrOffset if graph.control(address).is_none() => {
                address = graph.input(address, 0);
            }
            _ => return None,
        }
    }
    None
}

/// Reject explicit poison, restricted representations and unknown pure value
/// producers. Fixed integer snapshots retain their existing L contract; this
/// is not a new global definedness analysis or a reason to move a fixed read.
fn ordinary_value(
    graph: &FunctionGraph,
    node: NodeId,
    seen: &mut HashSet<NodeId>,
    fuel: &mut usize,
) -> bool {
    let node = graph.resolve(node);
    if seen.contains(&node) {
        return true;
    }
    if *fuel == 0
        || graph
            .rust_type(node)
            .and_then(|ty| integer(graph, ty))
            .is_none()
    {
        return false;
    }
    *fuel -= 1;
    if crate::ArithmeticFlags::allowed(graph.op(node)).is_some() && graph.payload(node) != 0
        || crate::IntegerIntrinsicPayload::may_produce_poison(graph.op(node), graph.payload(node))
    {
        return false;
    }
    let valid = match graph.op(node) {
        Opcode::Const => true,
        Opcode::Projection if graph.payload(node) >= 2 => matches!(
            graph.op(graph.input(node, 0)),
            Opcode::Start | Opcode::Load | Opcode::Call | Opcode::CallIndirect
        ),
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::BitAnd
        | Opcode::BitOr
        | Opcode::BitXor
        | Opcode::ShiftLeft
        | Opcode::ShiftRightLogical
        | Opcode::ShiftRightArithmetic
        | Opcode::BitNot
        | Opcode::Negate
        | Opcode::IntCast
        | Opcode::BitCount
        | Opcode::ByteSwap
        | Opcode::BitReverse
        | Opcode::FunnelShift
        | Opcode::SaturatingAdd
        | Opcode::SaturatingSub
            if graph.control(node).is_none() =>
        {
            graph
                .inputs(node)
                .iter()
                .all(|&input| ordinary_value(graph, input, seen, fuel))
        }
        _ => false,
    };
    if valid {
        seen.insert(node);
    }
    valid
}

fn plan(graph: &FunctionGraph, uses: &Uses, copy: Copy) -> Option<Plan> {
    let root = allocation(graph, copy.source)?;
    // Only this call's source operand may consume an address. A destination
    // alias, another copy, a raw address value or any unknown call rejects it.
    let accesses = collect_with_copy(graph, uses, root, Some((copy.node, copy.bytes)))?;
    let range = *accesses.get(&copy.node)?;
    let mut values = Vec::with_capacity(copy.bytes as usize);
    let mut seen = HashSet::default();
    let mut fuel = 512;
    for offset in range.offset..range.offset + range.bytes {
        let lane = Access { offset, bytes: 1 };
        let stored = writer(
            graph,
            graph.input(copy.node, 0),
            lane,
            &accesses,
            &mut HashSet::default(),
            &mut 256,
        )?;
        // forward's same-type shortcut also supports restricted typed loads;
        // this copy path deliberately requires ordinary integer storage first.
        integer(graph, graph.rust_type(stored.value)?)?;
        if !ordinary_value(graph, stored.value, &mut seen, &mut fuel) {
            return None;
        }
        values.push(forward(graph, stored, lane, copy.byte)?);
    }
    Some(Plan { copy, values })
}

impl Plan {
    fn apply(self, graph: &mut FunctionGraph, uses: &Uses) {
        let copy = self.copy;
        // Finish every source snapshot before changing destination memory.
        let values: Vec<_> = chunks::collect(graph, &self.values)
            .into_iter()
            .map(|chunk| (chunk.offset, chunk.value.materialize(graph)))
            .collect();
        let mut control = graph.control(copy.node);
        let mut effect = graph.input(copy.node, 0);
        let pointer_ty = graph.value_type(copy.destination).clone();
        for (offset, value) in values {
            let offset = graph.add_constant(offset as u128, copy.count_ty);
            let address = graph.add_node(
                Opcode::PtrOffset,
                NodeId::NONE,
                &[copy.destination, offset],
                0,
                pointer_ty.clone(),
            );
            let store = graph.add_node(
                Opcode::Store,
                control,
                &[effect, address, value],
                0, // Unknown until the normal post-layout partition rebuild.
                ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
            );
            control = graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[store],
                0,
                ValueType::Control,
            );
            effect = graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[store],
                1,
                ValueType::Effect,
            );
        }
        for u in uses.of(copy.node) {
            let replacement = match graph.payload(u.user) {
                0 => control,
                1 => effect,
                2 => copy.destination,
                _ => unreachable!("checked memcpy projection"),
            };
            graph.replace(u.user, replacement);
        }
        graph.remove_node(copy.node);
    }
}

pub(super) fn run(graph: &mut FunctionGraph) {
    let uses = Uses::new(graph);
    let plans: Vec<_> = graph
        .ids()
        .filter_map(|node| descriptor(graph, node, &uses))
        .filter_map(|copy| plan(graph, &uses, copy))
        .collect();
    for plan in plans {
        plan.apply(graph, &uses);
    }
    graph.resolve_edges();
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
