//! Collapse effect-free boolean diamonds, retaining the branch's poison
//! observation as RequireDefined. Never speculate operations from either arm.
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::{TypeOracle, TypeShape};

pub(crate) fn run(graph: &mut FunctionGraph) -> usize {
    if !matches!(graph.level, GraphLevel::Specialized | GraphLevel::Layout) {
        return 0;
    }
    let uses = Uses::new(graph);
    let regions: Vec<_> = graph
        .ids()
        .filter(|&n| graph.op(n) == Opcode::Region)
        .collect();
    let mut changed = 0;
    for region in regions {
        let edges = graph.inputs(region).to_vec();
        if edges.len() != 2 || edges.iter().any(|&n| graph.op(n) != Opcode::Jump) {
            continue;
        }
        let exits: Vec<_> = edges.iter().map(|&n| graph.control(n)).collect();
        if exits.iter().any(|&n| graph.op(n) != Opcode::Projection) {
            continue;
        }
        let branch = graph.input(exits[0], 0);
        if graph.op(branch) != Opcode::If
            || graph.input(exits[1], 0) != branch
            || graph.payload(exits[0]) > 1
            || graph.payload(exits[1]) != 1 - graph.payload(exits[0])
        {
            continue;
        }
        // The only uses of the branch exits are their direct merge jumps.
        // This excludes witnesses, anchored arithmetic, effects and other arms.
        if uses.of(branch).len() != 2
            || exits.iter().zip(&edges).any(|(&exit, &edge)| {
                uses.of(exit)
                    .iter()
                    .any(|u| u.user != edge || u.kind != UseKind::Control)
                    || uses.of(edge).iter().any(|u| u.user != region)
            })
        {
            continue;
        }
        let condition = graph.resolve(graph.input(branch, 0));
        let mut phis = Vec::new();
        let mut effect = None;
        let mut valid = true;
        for usage in uses.of(region) {
            let phi = usage.user;
            if graph.op(phi) != Opcode::Phi {
                continue;
            }
            let inputs = graph.inputs(phi);
            if inputs.len() != 3 || inputs[0] != region {
                valid = false;
                break;
            }
            let a = graph.resolve(inputs[1]);
            let b = graph.resolve(inputs[2]);
            if a == b && a != phi {
                if graph.value_type(phi) == &ValueType::Effect {
                    effect = Some(a);
                }
                phis.push((phi, Some(a), false));
            } else if graph.value_type(phi) == graph.value_type(condition)
                && graph
                    .rust_type(phi)
                    .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
                && graph.op(a) == Opcode::Const
                && graph.op(b) == Opcode::Const
                && graph.constant_value(a) <= 1
                && graph.constant_value(b) == 1 - graph.constant_value(a)
            {
                let invert = graph.constant_value(a) == u128::from(graph.payload(exits[0]));
                phis.push((phi, None, invert));
            } else {
                valid = false;
                break;
            }
        }
        let Some(effect) = effect else { continue };
        if !valid {
            continue;
        }
        let (control, observed_effect) = if graph.level == GraphLevel::Layout {
            // L is already a published refinement: poison-only executions
            // need not preserve S's diagnostic (as in erase_requirements).
            (graph.control(branch), effect)
        } else {
            let observe = graph.add_node(
                Opcode::RequireDefined,
                graph.control(branch),
                &[effect, condition],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
            );
            let control = graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[observe],
                0,
                ValueType::Control,
            );
            let observed_effect = graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[observe],
                1,
                ValueType::Effect,
            );
            (control, observed_effect)
        };
        for (phi, same, invert) in phis {
            let replacement = if graph.value_type(phi) == &ValueType::Effect {
                observed_effect
            } else if let Some(same) = same {
                same
            } else if invert {
                graph.add_node(
                    Opcode::BitNot,
                    NodeId::NONE,
                    &[condition],
                    0,
                    graph.value_type(condition).clone(),
                )
            } else {
                condition
            };
            graph.replace(phi, replacement);
        }
        graph.replace(region, control);
        for node in edges.into_iter().chain(exits).chain([branch]) {
            graph.remove_node(node);
        }
        changed += 1;
    }
    graph.resolve_edges();
    changed
}
