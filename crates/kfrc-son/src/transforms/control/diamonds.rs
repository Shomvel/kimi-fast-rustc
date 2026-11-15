//! Collapse effect-free boolean diamonds, retaining the branch's poison
//! observation as RequireDefined. Never speculate operations from either arm.
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::{TypeOracle, TypeShape};

pub(crate) fn run(graph: &mut FunctionGraph) -> usize {
    if !matches!(graph.level, GraphLevel::Specialized | GraphLevel::Layout) {
        return 0;
    }
    // Built at the first structural candidate, which precedes every edit, so
    // this is the same snapshot as building it on entry.
    let mut uses: Option<Uses> = None;
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
        let uses = uses.get_or_insert_with(|| Uses::new(graph));
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
        if !valid {
            continue;
        }
        let (control, observed_effect) = if graph.level == GraphLevel::Layout {
            // L is already a published refinement: poison-only executions
            // need not preserve S's diagnostic (as in erase_requirements).
            (graph.control(branch), effect)
        } else {
            // S must keep the branch's poison observation in the effect
            // chain. L has already published that refinement and an empty
            // diamond needs no effect Phi to describe its unchanged memory.
            let Some(effect) = effect else { continue };
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
            (control, Some(observed_effect))
        };
        for (phi, same, invert) in phis {
            let replacement = if graph.value_type(phi) == &ValueType::Effect {
                observed_effect.expect("an effect Phi supplied the incoming effect")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComparePredicate, Interpreter, Outcome};
    use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign};

    #[test]
    fn layout_empty_diamond_needs_no_redundant_effect_phi() {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let mut graph = FunctionGraph::new(
            "empty_diamond",
            FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Layout;
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
            ]),
        );
        graph.start = start;
        let control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            0,
            ValueType::Control,
        );
        let effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            1,
            ValueType::Effect,
        );
        let value = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(word),
        );
        let zero = graph.add_constant(0, word);
        let condition = graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[value, zero],
            ComparePredicate::Equal as u32,
            ValueType::Rust(boolean),
        );
        let branch = graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let jumps: Vec<_> = (0..2)
            .map(|side| {
                let exit = graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[branch],
                    side,
                    ValueType::Control,
                );
                graph.add_node(Opcode::Jump, exit, &[], 0, ValueType::Control)
            })
            .collect();
        let region = graph.add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
        graph.add_node(
            Opcode::Return,
            region,
            &[effect, value],
            0,
            ValueType::Control,
        );
        crate::verify_graph(&mut graph).unwrap();
        let original = graph.clone();
        assert_eq!(super::run(&mut graph), 1);
        crate::verify_graph(&mut graph).unwrap();
        for input in [0, 1, 7, u64::MAX] {
            let before =
                Interpreter::new(&original).run(&[input], &mut |_: &str, _: &[u64]| unreachable!());
            let after =
                Interpreter::new(&graph).run(&[input], &mut |_: &str, _: &[u64]| unreachable!());
            assert_eq!(before, Ok(Outcome::Return(input)));
            assert_eq!(after, before);
        }
    }
}
