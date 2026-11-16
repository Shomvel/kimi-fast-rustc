//! Collapse effect-free diamonds. S retains the branch's poison observation;
//! L can also select a proved total integer expression. Fixed operations keep
//! their original control paths.
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::{TypeOracle, TypeShape};

#[path = "diamond_values.rs"]
mod values;

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
        if graph.is_dead(region) {
            continue;
        }
        let edges = graph.inputs(region).to_vec();
        if edges.len() != 2
            || edges
                .iter()
                .any(|&n| graph.op(n) != Opcode::Jump || graph.is_dead(n))
        {
            continue;
        }
        // Inlining often leaves empty Jump/Region wrappers on one arm.
        // Trace both arms using the same exact-use proof as a direct diamond.
        let uses = uses.get_or_insert_with(|| Uses::new(graph));
        let Some(left) = Arm::new(graph, uses, edges[0], region) else {
            continue;
        };
        let Some(right) = Arm::new(graph, uses, edges[1], region) else {
            continue;
        };
        let exits = [left.exit, right.exit];
        let branch = graph.input(exits[0], 0);
        if graph.op(branch) != Opcode::If
            || graph.is_dead(branch)
            || graph.input(exits[1], 0) != branch
            || graph.payload(exits[0]) > 1
            || graph.payload(exits[1]) != 1 - graph.payload(exits[0])
        {
            continue;
        }
        if uses.of(branch).len() != 2 {
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
                phis.push((phi, Some(values::Replacement::Existing(a)), false));
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
            } else if let Some(value) =
                values::replacement(graph, condition, graph.payload(exits[0]) == 0, a, b)
            {
                phis.push((phi, Some(value), false));
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
                same.materialize(graph)
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
        for node in left
            .controls
            .into_iter()
            .chain(right.controls)
            .chain(exits)
            .chain([branch])
        {
            graph.remove_node(node);
        }
        changed += 1;
    }
    graph.resolve_edges();
    changed
}

struct Arm {
    exit: NodeId,
    controls: Vec<NodeId>,
}

impl Arm {
    fn new(
        graph: &FunctionGraph,
        uses: &Uses,
        mut jump: NodeId,
        mut target: NodeId,
    ) -> Option<Self> {
        let mut controls = Vec::new();
        static WRAPPERS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let wrappers =
            *WRAPPERS.get_or_init(|| std::env::var("KFRC_DIAMOND_WRAPPERS").as_deref() != Ok("0"));
        for _ in 0..if wrappers { 8 } else { 1 } {
            if graph.is_dead(jump)
                || graph.op(jump) != Opcode::Jump
                || !graph.inputs(jump).is_empty()
            {
                return None;
            }
            let [usage] = uses.of(jump) else { return None };
            if usage.user != target || !matches!(usage.kind(), UseKind::Operand(_)) {
                return None;
            }
            controls.push(jump);
            let source = graph.control(jump);
            if source.is_none() || graph.is_dead(source) {
                return None;
            }
            let [usage] = uses.of(source) else {
                return None;
            };
            if usage.user != jump || usage.kind() != UseKind::Control {
                return None;
            }
            if graph.op(source) == Opcode::Projection {
                return Some(Self {
                    exit: source,
                    controls,
                });
            }
            // Identity Phis and fixed or anchored users are excluded. The
            // wrapper contributes only one incoming edge and one outgoing jump.
            if graph.op(source) != Opcode::Region || graph.inputs(source).len() != 1 {
                return None;
            }
            controls.push(source);
            target = source;
            jump = graph.input(source, 0);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComparePredicate, Interpreter, Outcome, verify_graph};
    use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign};

    pub(super) fn empty_diamond(wrappers: usize, anchored: bool) -> FunctionGraph {
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
                let mut exit = graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[branch],
                    side,
                    ValueType::Control,
                );
                for _ in 0..wrappers * (side as usize + 1) {
                    let jump = graph.add_node(Opcode::Jump, exit, &[], 0, ValueType::Control);
                    exit = graph.add_node(
                        Opcode::Region,
                        NodeId::NONE,
                        &[jump],
                        0,
                        ValueType::Control,
                    );
                }
                if anchored && side == 0 {
                    graph.add_node(Opcode::Add, exit, &[value, zero], 0, ValueType::Rust(word));
                }
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
        graph
    }

    #[test]
    fn layout_empty_diamonds_need_no_effect_phi_and_reject_anchored_uses() {
        for wrappers in [0, 1, 3] {
            for anchored in [false, true] {
                let mut graph = empty_diamond(wrappers, anchored);
                verify_graph(&mut graph).unwrap();
                let original = graph.clone();
                assert_eq!(super::run(&mut graph), usize::from(!anchored));
                verify_graph(&mut graph).unwrap();
                for input in [0, 1, 7, u64::MAX] {
                    let before = Interpreter::new(&original)
                        .run(&[input], &mut |_: &str, _: &[u64]| unreachable!());
                    let after = Interpreter::new(&graph)
                        .run(&[input], &mut |_: &str, _: &[u64]| unreachable!());
                    assert_eq!(before, Ok(Outcome::Return(input)));
                    assert_eq!(after, before);
                }
            }
        }
    }
}
