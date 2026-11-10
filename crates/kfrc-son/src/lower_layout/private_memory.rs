//! Forward exact private lanes after layout exposes stack round trips. A slot
//! is erased only if every use is an in-bounds scalar access or lifetime marker
//! and every load has a writer on all effect paths. Cycles conservatively fail.
use super::*;
use crate::analysis::uses::{UseKind, Uses};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Access {
    offset: u64,
    bytes: u64,
}

fn collect(graph: &FunctionGraph, uses: &Uses, root: NodeId) -> Option<HashMap<NodeId, Access>> {
    let size = graph.stack_slots[graph.payload(root) as usize].size;
    let mut pending = vec![(root, 0u64)];
    let mut seen = HashSet::default();
    let mut accesses = HashMap::default();
    while let Some((address, offset)) = pending.pop() {
        if !seen.insert(address) {
            continue;
        }
        for u in uses.of(address) {
            let node = u.user;
            match (graph.op(node), u.kind) {
                (Opcode::PtrOffset, UseKind::Operand(0)) => {
                    let displacement = graph.input(node, 1);
                    if graph.op(displacement) != Opcode::Const {
                        return None;
                    }
                    let offset = offset
                        .checked_add(u64::try_from(graph.constant_value(displacement)).ok()?)?;
                    if offset > size {
                        return None;
                    }
                    pending.push((node, offset));
                }
                (Opcode::Load | Opcode::Store, UseKind::Operand(1)) => {
                    let ty = if graph.op(node) == Opcode::Store {
                        graph.rust_type(graph.input(node, 2))?
                    } else {
                        let ValueType::Tuple(fields) = graph.value_type(node) else {
                            return None;
                        };
                        let ValueType::Rust(ty) = fields[2] else {
                            return None;
                        };
                        ty
                    };
                    let bytes = graph.oracle.size_align(ty)?.size;
                    if !matches!(bytes, 1 | 2 | 4 | 8) || offset.checked_add(bytes)? > size {
                        return None;
                    }
                    accesses.insert(node, Access { offset, bytes });
                }
                (Opcode::SlotLive | Opcode::SlotDead, UseKind::Operand(1)) => {
                    accesses.insert(
                        node,
                        Access {
                            offset: 0,
                            bytes: 0,
                        },
                    );
                }
                _ => return None,
            }
        }
    }
    Some(accesses)
}

fn writer(
    graph: &FunctionGraph,
    effect: NodeId,
    lane: Access,
    accesses: &HashMap<NodeId, Access>,
    active: &mut HashSet<NodeId>,
    fuel: &mut usize,
) -> Option<NodeId> {
    let effect = graph.resolve(effect);
    if *fuel == 0 || !active.insert(effect) {
        return None;
    }
    *fuel -= 1;
    let value = if graph.op(effect) == Opcode::Phi {
        let mut result = None;
        for &incoming in &graph.inputs(effect)[1..] {
            let value = writer(graph, incoming, lane, accesses, active, fuel)?;
            if result.is_some_and(|old| old != value) {
                return None;
            }
            result = Some(value);
        }
        result
    } else if graph.op(effect) == Opcode::Projection && graph.payload(effect) == 1 {
        let source = graph.input(effect, 0);
        if graph.op(source) == Opcode::Start {
            None
        } else if let Some(access) = accesses.get(&source) {
            match graph.op(source) {
                Opcode::SlotLive | Opcode::SlotDead => None,
                Opcode::Store if *access == lane => Some(graph.resolve(graph.input(source, 2))),
                Opcode::Store
                    if access.offset < lane.offset + lane.bytes
                        && lane.offset < access.offset + access.bytes =>
                {
                    None
                }
                _ => writer(graph, graph.input(source, 0), lane, accesses, active, fuel),
            }
        } else if graph
            .inputs(source)
            .first()
            .is_some_and(|n| graph.value_type(*n) == &ValueType::Effect)
        {
            // No pointer to this allocation escapes, including through calls.
            writer(graph, graph.input(source, 0), lane, accesses, active, fuel)
        } else {
            None
        }
    } else {
        None
    };
    active.remove(&effect);
    value
}

pub(super) fn run(graph: &mut FunctionGraph) {
    graph.resolve_edges();
    let uses = Uses::new(graph);
    let mut rewrites = Vec::new();
    for root in graph.ids().filter(|&n| graph.op(n) == Opcode::StackSlot) {
        let Some(accesses) = collect(graph, &uses, root) else {
            continue;
        };
        let mut values = HashMap::default();
        let mut valid = true;
        for (&node, &lane) in &accesses {
            if graph.op(node) != Opcode::Load {
                continue;
            }
            let Some(value) = writer(
                graph,
                graph.input(node, 0),
                lane,
                &accesses,
                &mut HashSet::default(),
                &mut 256,
            ) else {
                valid = false;
                break;
            };
            let ValueType::Tuple(fields) = graph.value_type(node) else {
                unreachable!()
            };
            if &fields[2] != graph.value_type(value) {
                valid = false;
                break;
            }
            values.insert(node, value);
        }
        if valid {
            rewrites.extend(
                accesses
                    .keys()
                    .map(|&node| (node, values.get(&node).copied())),
            );
        }
    }
    for (node, value) in rewrites {
        for u in uses.of(node) {
            let replacement = match graph.payload(u.user) {
                0 => graph.control(node),
                1 => graph.input(node, 0),
                2 => value.expect("load writer"),
                _ => unreachable!(),
            };
            graph.replace(u.user, replacement);
        }
        graph.remove_node(node);
    }
    graph.resolve_edges();
    graph.compact();
}

#[cfg(test)]
#[path = "private_memory_tests.rs"]
mod tests;
