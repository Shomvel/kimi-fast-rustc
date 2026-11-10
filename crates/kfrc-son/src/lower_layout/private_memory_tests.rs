use super::*;
use crate::{CallTarget, GraphLevel, Interpreter, Outcome, verify_graph};
use kfrc_body::{FunctionAbi, PassMode, RecordedTypes};

fn fixed(
    g: &mut FunctionGraph,
    control: &mut NodeId,
    effect: &mut NodeId,
    op: Opcode,
    operands: &[NodeId],
    payload: u32,
    result: Option<TyId>,
) -> Option<NodeId> {
    let mut inputs = vec![*effect];
    inputs.extend_from_slice(operands);
    let mut outputs = vec![ValueType::Control, ValueType::Effect];
    outputs.extend(result.map(ValueType::Rust));
    let n = g.add_node(op, *control, &inputs, payload, ValueType::Tuple(outputs));
    *control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[n],
        0,
        ValueType::Control,
    );
    *effect = g.add_node(Opcode::Projection, NodeId::NONE, &[n], 1, ValueType::Effect);
    result.map(|ty| {
        g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[n],
            2,
            ValueType::Rust(ty),
        )
    })
}

// The load crosses a branch, an unrelated call and an effect Phi. A differing
// writer or an escaped address must keep the memory rather than invent a Phi.
#[test]
fn joins_and_calls_forward_only_identical_unescaped_writers() {
    for different in [false, true] {
        for escape in [false, true] {
            let mut types = RecordedTypes::new();
            let word = types.intern(
                TypeShape::Int {
                    width: IntegerWidth::W64,
                    signed: false,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
            let pointer = types.intern(
                TypeShape::RawPointer {
                    mutability: kfrc_body::Mutability::Mut,
                    pointee: word,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            let mut g = FunctionGraph::new(
                "private_join",
                FunctionAbi {
                    args: vec![PassMode::Direct],
                    ret: PassMode::Direct,
                    c_variadic: false,
                },
                types,
            );
            g.level = GraphLevel::Layout;
            let start = g.add_node(
                Opcode::Start,
                NodeId::NONE,
                &[],
                0,
                ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(boolean),
                ]),
            );
            g.start = start;
            let mut control = g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[start],
                0,
                ValueType::Control,
            );
            let mut effect = g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[start],
                1,
                ValueType::Effect,
            );
            let flag = g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[start],
                2,
                ValueType::Rust(boolean),
            );
            g.stack_slots.push(SizeAlign { size: 8, align: 8 });
            let slot = g.add_node(
                Opcode::StackSlot,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(pointer),
            );
            let original = g.add_constant(37, word);
            fixed(
                &mut g,
                &mut control,
                &mut effect,
                Opcode::Store,
                &[slot, original],
                0,
                None,
            );
            let branch = g.add_node(
                Opcode::If,
                control,
                &[flag],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
            );
            let mut yes = g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                0,
                ValueType::Control,
            );
            let no = g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                1,
                ValueType::Control,
            );
            let mut yes_effect = effect;
            let target = g.add_callee(CallTarget {
                symbol: "unrelated".into(),
                abi: FunctionAbi {
                    args: if escape {
                        vec![PassMode::Direct]
                    } else {
                        vec![]
                    },
                    ret: PassMode::Direct,
                    c_variadic: false,
                },
            });
            fixed(
                &mut g,
                &mut yes,
                &mut yes_effect,
                Opcode::Call,
                if escape {
                    std::slice::from_ref(&slot)
                } else {
                    &[]
                },
                target,
                Some(word),
            );
            let value = if different {
                g.add_constant(91, word)
            } else {
                original
            };
            fixed(
                &mut g,
                &mut yes,
                &mut yes_effect,
                Opcode::Store,
                &[slot, value],
                0,
                None,
            );
            let left = g.add_node(Opcode::Jump, yes, &[], 0, ValueType::Control);
            let right = g.add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
            control = g.add_node(
                Opcode::Region,
                NodeId::NONE,
                &[left, right],
                0,
                ValueType::Control,
            );
            effect = g.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &[control, yes_effect, effect],
                0,
                ValueType::Effect,
            );
            let value = fixed(
                &mut g,
                &mut control,
                &mut effect,
                Opcode::Load,
                &[slot],
                0,
                Some(word),
            )
            .unwrap();
            g.add_node(
                Opcode::Return,
                control,
                &[effect, value],
                0,
                ValueType::Control,
            );
            verify_graph(&mut g).unwrap();
            let before = g.clone();
            run(&mut g);
            verify_graph(&mut g).unwrap();
            assert_eq!(
                g.ids().any(|n| g.op(n) == Opcode::Load),
                different || escape
            );
            for selector in [0, 1] {
                let mut outcomes = vec![];
                for graph in [&before, &g] {
                    let mut calls = 0;
                    let outcome = Interpreter::new(graph)
                        .run(&[selector], &mut |name: &str, _: &[u64]| {
                            assert_eq!(name, "unrelated");
                            calls += 1;
                            Ok(0)
                        })
                        .unwrap();
                    outcomes.push((outcome, calls));
                }
                assert_eq!(outcomes[0], outcomes[1]);
                assert_eq!(
                    outcomes[1],
                    (
                        Outcome::Return(if selector == 1 && different { 91 } else { 37 }),
                        selector
                    )
                );
            }
        }
    }
}
