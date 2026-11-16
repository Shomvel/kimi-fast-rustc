//! Consume an ABI-checked no-return proof without removing the call's effects.
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, NodeId, Opcode, ValueType};

/// Disconnect each normal continuation and retain one terminal control exit.
/// The caller immediately prunes unreachable CFG/Phi slots before verification.
pub(crate) fn run(graph: &mut FunctionGraph) -> usize {
    let calls: Vec<_> = graph
        .ids()
        .filter(|&node| {
            graph.op(node) == Opcode::Call
                && graph
                    .callee_attrs(graph.payload(node))
                    .is_some_and(|attrs| attrs.noreturn)
        })
        .collect();
    if calls.is_empty() {
        return 0;
    }
    let uses = Uses::new(graph);
    let mut changed = 0;
    for call in calls {
        let controls: Vec<_> = uses
            .of(call)
            .iter()
            .filter_map(|usage| {
                let node = usage.user;
                (usage.kind() == UseKind::Operand(0)
                    && graph.op(node) == Opcode::Projection
                    && graph.payload(node) == 0)
                    .then_some(node)
            })
            .collect();
        if !controls.is_empty()
            && controls.iter().all(|&node| {
                !uses.of(node).is_empty()
                    && uses.of(node).iter().all(|usage| {
                        usage.kind() == UseKind::Control
                            && graph.op(usage.user) == Opcode::Unreachable
                    })
            })
        {
            continue;
        }
        for control in controls {
            graph.remove_node(control);
        }
        let terminal = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            0,
            ValueType::Control,
        );
        graph.add_node(Opcode::Unreachable, terminal, &[], 0, ValueType::Control);
        changed += 1;
    }
    changed
}
