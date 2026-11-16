//! Pack adjacent constant byte stores to a known stack allocation. The only
//! layout target is little-endian x86_64, which permits unaligned scalar
//! stores. Each packed store covers exactly the original initialized bytes.
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::{IntegerWidth, SizeAlign, TypeOracle, TypeShape};

pub(super) fn run(graph: &mut FunctionGraph) -> usize {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if graph.level != GraphLevel::Layout
        || !*ENABLED.get_or_init(|| std::env::var("KFRC_CONSTANT_STORES").as_deref() != Ok("0"))
    {
        return 0;
    }
    let stores: Vec<_> = graph
        .ids()
        .filter(|&n| graph.op(n) == Opcode::Store)
        .collect();
    let mut uses = None;
    let mut removed = 0;
    for first in stores {
        if graph.is_dead(first) || byte(graph, first).is_none() {
            continue;
        }
        let Some((root, offset)) = stack_address(graph, graph.input(first, 1)) else {
            continue;
        };
        let uses = uses.get_or_insert_with(|| Uses::new(graph));
        let mut chain = vec![first];
        let mut outgoing = Vec::new();
        while chain.len() < 8 {
            let current = *chain.last().unwrap();
            let Some(projections) = projections(graph, uses, current) else {
                break;
            };
            let [control_user] = uses.of(projections[0]) else {
                break;
            };
            let [effect_user] = uses.of(projections[1]) else {
                break;
            };
            let next = control_user.user;
            if control_user.kind != UseKind::Control
                || effect_user.kind != UseKind::Operand(0)
                || effect_user.user != next
                || graph.is_dead(next)
                || graph.op(next) != Opcode::Store
                || graph.control(next) != projections[0]
                || graph.input(next, 0) != projections[1]
                || graph.payload(next) != graph.payload(first)
                || byte(graph, next).is_none()
                || stack_address(graph, graph.input(next, 1))
                    != Some((root, offset + chain.len() as u64))
            {
                break;
            }
            outgoing.push(projections);
            chain.push(next);
        }
        let bytes = [8, 4, 2].into_iter().find(|&n| n <= chain.len());
        let Some(bytes) = bytes else { continue };
        let width = match bytes {
            2 => IntegerWidth::W16,
            4 => IntegerWidth::W32,
            8 => IntegerWidth::W64,
            _ => unreachable!(),
        };
        let ty = graph.oracle.intern(
            TypeShape::Int {
                width,
                signed: false,
            },
            Some(SizeAlign {
                size: bytes as u64,
                align: bytes as u64,
            }),
        );
        if graph.oracle.scalar_validity(ty).is_some() {
            continue;
        }
        let bits = chain[..bytes]
            .iter()
            .enumerate()
            .fold(0u64, |bits, (index, &store)| {
                bits | u64::from(byte(graph, store).unwrap()) << (8 * index)
            });
        let constant = graph.add_constant(u128::from(bits), ty);
        let last = chain[bytes - 1];
        let control = graph.control(first);
        let effect = graph.input(first, 0);
        let address = graph.input(first, 1);
        graph.set_control(last, control);
        graph.set_input(last, 0, effect);
        graph.set_input(last, 1, address);
        graph.set_input(last, 2, constant);
        // Intermediate observations have exactly one control/effect consumer,
        // the following store. No load, call, branch or pinned value is skipped.
        for (&store, projections) in chain[..bytes - 1].iter().zip(&outgoing) {
            graph.remove_node(store);
            for &projection in projections {
                graph.remove_node(projection);
            }
        }
        removed += bytes - 1;
    }
    removed
}

fn byte(graph: &FunctionGraph, store: NodeId) -> Option<u8> {
    let value = graph.resolve(graph.input(store, 2));
    let ty = graph.rust_type(value)?;
    (graph.op(value) == Opcode::Const
        && graph.control(value).is_none()
        && matches!(
            graph.oracle.shape(ty),
            TypeShape::Int {
                width: IntegerWidth::W8,
                ..
            }
        )
        && graph.oracle.scalar_validity(ty).is_none()
        && graph.oracle.size_align(ty)?.size == 1)
        .then(|| u8::try_from(graph.constant_value(value)).ok())
        .flatten()
}

fn projections(graph: &FunctionGraph, uses: &Uses, store: NodeId) -> Option<[NodeId; 2]> {
    let mut result = [NodeId::NONE; 2];
    if uses.of(store).len() != 2 {
        return None;
    }
    for usage in uses.of(store) {
        let node = usage.user;
        if usage.kind != UseKind::Operand(0)
            || graph.op(node) != Opcode::Projection
            || graph.is_dead(node)
            || graph.payload(node) > 1
            || graph.value_type(node)
                != &[ValueType::Control, ValueType::Effect][graph.payload(node) as usize]
            || !result[graph.payload(node) as usize].is_none()
        {
            return None;
        }
        result[graph.payload(node) as usize] = node;
    }
    Some(result)
}

fn stack_address(graph: &FunctionGraph, mut address: NodeId) -> Option<(NodeId, u64)> {
    let mut offset = 0u64;
    for _ in 0..16 {
        address = graph.resolve(address);
        if !graph.control(address).is_none() {
            return None;
        }
        match graph.op(address) {
            Opcode::StackSlot => {
                let slot = graph.stack_slots.get(graph.payload(address) as usize)?;
                return (offset < slot.size).then_some((address, offset));
            }
            Opcode::PtrOffset => {
                let displacement = graph.resolve(graph.input(address, 1));
                if graph.op(displacement) != Opcode::Const {
                    return None;
                }
                let TypeShape::Int { width, signed } =
                    graph.oracle.shape(graph.rust_type(displacement)?)
                else {
                    return None;
                };
                let bits = width.bits(64);
                let raw = graph.constant_value(displacement);
                if bits > 64
                    || raw >= (1u128 << bits)
                    || (*signed && raw & (1u128 << (bits - 1)) != 0)
                {
                    return None;
                }
                offset = offset.checked_add(u64::try_from(raw).ok()?)?;
                address = graph.input(address, 0);
            }
            Opcode::PointerCast => address = graph.input(address, 0),
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
#[path = "constant_stores_tests.rs"]
mod tests;
