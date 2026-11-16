use super::*;

fn scalar_bytes(graph: &FunctionGraph, ty: TyId) -> Option<u64> {
    let bits = match graph.oracle.shape(ty) {
        TypeShape::Int { width, .. } if width.bits(64) <= 64 => width.bits(64),
        TypeShape::Bool => 8,
        TypeShape::Char => 32,
        TypeShape::RawPointer { pointee, .. } | TypeShape::Reference { pointee, .. }
            if !graph.oracle.semantic().requires_pointer_metadata(*pointee) =>
        {
            64
        }
        _ => return None,
    };
    let bytes = u64::from(bits / 8);
    (graph.oracle.size_align(ty)?.size == bytes).then_some(bytes)
}

pub(super) fn slot(
    graph: &FunctionGraph,
    uses: &Uses,
    cfg: &ControlFlow,
    identity: u32,
    roots: &[NodeId],
    fuel: &mut usize,
) -> Option<Slot> {
    let size = graph.stack_slots[identity as usize].size;
    let mut pending: Vec<_> = roots.iter().map(|&node| (node, 0u64)).collect();
    let mut seen = HashMap::default();
    let mut accesses = BTreeMap::new();
    let mut lanes = BTreeMap::new();
    while let Some((address, offset)) = pending.pop() {
        if let Some(old) = seen.insert(address, offset) {
            if old != offset {
                return None;
            }
            continue;
        }
        for usage in uses.of(address) {
            *fuel = fuel.checked_sub(1)?;
            let node = usage.user;
            match (graph.op(node), usage.kind()) {
                (Opcode::PtrOffset, UseKind::Operand(0)) if graph.control(node).is_none() => {
                    let step = graph.input(node, 1);
                    if graph.op(step) != Opcode::Const
                        || !matches!(graph.oracle.shape(graph.rust_type(step)?), TypeShape::Int { width, .. } if width.bits(64) == 64)
                    {
                        return None;
                    }
                    let step = u64::try_from(graph.constant_value(step)).ok()? as i64;
                    let offset = offset.checked_add_signed(step)?;
                    if offset > size {
                        return None;
                    }
                    pending.push((node, offset));
                }
                (
                    op @ (Opcode::Load | Opcode::Store | Opcode::SlotLive | Opcode::SlotDead),
                    UseKind::Operand(1),
                ) => {
                    if !cfg.contains(node) {
                        return None;
                    }
                    let max = if op == Opcode::Load { 2 } else { 1 };
                    if uses.of(node).iter().any(|usage| {
                        usage.kind() != UseKind::Operand(0)
                            || graph.op(usage.user) != Opcode::Projection
                            || graph.payload(usage.user) > max
                    }) {
                        return None;
                    }
                    let lane = if matches!(op, Opcode::SlotLive | Opcode::SlotDead) {
                        if offset != 0 {
                            return None;
                        }
                        None
                    } else {
                        let ty = if op == Opcode::Store {
                            graph.rust_type(graph.input(node, 2))?
                        } else {
                            let ValueType::Tuple(fields) = graph.value_type(node) else {
                                return None;
                            };
                            let ValueType::Rust(ty) = *fields.get(2)? else {
                                return None;
                            };
                            ty
                        };
                        let bytes = scalar_bytes(graph, ty)?;
                        if offset.checked_add(bytes)? > size {
                            return None;
                        }
                        let lane = Lane { offset, bytes, ty };
                        lanes.insert(lane, 0usize);
                        Some(lane)
                    };
                    accesses.insert(node, lane);
                }
                // Unknown calls, pointer Phis, stored/returned/compared
                // addresses and by-value ABI transfers retain storage.
                _ => return None,
            }
        }
    }
    if lanes.is_empty() || lanes.len() > 64 {
        return None;
    }
    let mut end = 0;
    for (index, (lane, number)) in lanes.iter_mut().enumerate() {
        if lane.offset < end {
            return None;
        }
        end = lane.offset + lane.bytes;
        *number = index;
    }
    Some(Slot {
        accesses: accesses
            .into_iter()
            .map(|(node, lane)| (node, lane.map(|lane| lanes[&lane])))
            .collect(),
        lanes: lanes.into_keys().collect(),
    })
}
