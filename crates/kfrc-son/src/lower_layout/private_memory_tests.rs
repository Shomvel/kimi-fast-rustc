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

struct IntegerSlot {
    graph: FunctionGraph,
    control: NodeId,
    effect: NodeId,
    slot: NodeId,
    argument: NodeId,
    word: TyId,
}

impl IntegerSlot {
    fn new(width: IntegerWidth, signed: bool) -> Self {
        let mut types = RecordedTypes::new();
        let bits = width.bits(64);
        let word = types.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign {
                size: u64::from(bits / 8),
                align: 1,
            }),
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: kfrc_body::Mutability::Mut,
                pointee: word,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "private_integer",
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
        let argument = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(word),
        );
        graph.stack_slots.push(SizeAlign { size: 8, align: 1 });
        let slot = graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(pointer),
        );
        Self {
            graph,
            control,
            effect,
            slot,
            argument,
            word,
        }
    }

    fn integer(&mut self, width: IntegerWidth, signed: bool) -> TyId {
        self.graph.oracle.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign {
                size: u64::from(width.bits(64) / 8),
                align: 1,
            }),
        )
    }

    fn address(&mut self, offset: u64, ty: TyId) -> NodeId {
        let pointer = self.graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: kfrc_body::Mutability::Mut,
                pointee: ty,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let offset_ty = self.integer(IntegerWidth::W64, false);
        let displacement = self.graph.add_constant(u128::from(offset), offset_ty);
        self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[self.slot, displacement],
            0,
            ValueType::Rust(pointer),
        )
    }

    fn store(&mut self, offset: u64, value: NodeId) {
        let ty = self.graph.rust_type(value).unwrap();
        let address = self.address(offset, ty);
        fixed(
            &mut self.graph,
            &mut self.control,
            &mut self.effect,
            Opcode::Store,
            &[address, value],
            0,
            None,
        );
    }

    fn load(&mut self, offset: u64, ty: TyId) -> NodeId {
        let address = self.address(offset, ty);
        fixed(
            &mut self.graph,
            &mut self.control,
            &mut self.effect,
            Opcode::Load,
            &[address],
            0,
            Some(ty),
        )
        .unwrap()
    }

    fn finish(mut self, value: NodeId) -> FunctionGraph {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
        self.graph
    }
}

#[test]
fn private_integer_subranges_preserve_each_byte_and_signedness() {
    let widths = [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ];
    for source in widths {
        for target in widths {
            let source_bytes = u64::from(source.bits(64) / 8);
            let target_bytes = u64::from(target.bits(64) / 8);
            if target_bytes > source_bytes {
                continue;
            }
            for offset in 0..=source_bytes - target_bytes {
                for source_signed in [false, true] {
                    for target_signed in [false, true] {
                        let mut f = IntegerSlot::new(source, source_signed);
                        let ty = f.integer(target, target_signed);
                        f.store(0, f.argument);
                        let value = f.load(offset, ty);
                        let mut after = f.finish(value);
                        let before = after.clone();
                        run(&mut after);
                        verify_graph(&mut after).unwrap();
                        assert!(
                            !after.ids().any(|n| after.op(n) == Opcode::Load),
                            "{source:?}/{source_signed} -> {target:?}/{target_signed} at {offset}"
                        );
                        for input in [0, 1, 0x807f_ff00_55aa_fe01, 1 << 63, u64::MAX] {
                            let outcomes: Vec<_> = [&before, &after]
                                .into_iter()
                                .map(|g| {
                                    Interpreter::new(g)
                                        .run(&[input], &mut |_: &str, _: &[u64]| Ok(0))
                                        .unwrap()
                                })
                                .collect();
                            let mask = u64::MAX >> (64 - target.bits(64));
                            assert_eq!(
                                outcomes[0],
                                Outcome::Return((input >> (offset * 8)) & mask)
                            );
                            assert_eq!(outcomes[0], outcomes[1]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn subrange_snapshots_precede_later_stores_and_ignore_disjoint_bytes() {
    for overwrite in [false, true] {
        let mut f = IntegerSlot::new(IntegerWidth::W32, false);
        let byte = f.integer(IntegerWidth::W8, false);
        f.store(0, f.argument);
        let old = f.load(1, byte);
        let extra = f.graph.add_constant(0xef, byte);
        f.store(if overwrite { 1 } else { 7 }, extra);
        let new = f.load(1, byte);
        let result = f.graph.add_node(
            Opcode::BitXor,
            NodeId::NONE,
            &[old, new],
            0,
            ValueType::Rust(byte),
        );
        let mut g = f.finish(result);
        run(&mut g);
        verify_graph(&mut g).unwrap();
        assert!(!g.ids().any(|n| g.op(n) == Opcode::Load));
        for input in [0, 0x1234_5678, u64::MAX] {
            let expected = if overwrite {
                ((input >> 8) & 255) ^ 0xef
            } else {
                0
            };
            assert_eq!(
                Interpreter::new(&g)
                    .run(&[input], &mut |_: &str, _: &[u64]| Ok(0))
                    .unwrap(),
                Outcome::Return(expected)
            );
        }
    }
}

#[test]
fn partial_overwrite_or_uninitialized_bytes_keep_private_storage() {
    for overwrite in [false, true] {
        let mut f = IntegerSlot::new(IntegerWidth::W32, false);
        let byte = f.integer(IntegerWidth::W8, false);
        f.store(0, f.argument);
        if overwrite {
            let value = f.graph.add_constant(0x7f, byte);
            f.store(1, value);
        }
        let value = f.load(if overwrite { 0 } else { 1 }, f.word);
        let mut g = f.finish(value);
        let before = g.clone();
        run(&mut g);
        verify_graph(&mut g).unwrap();
        assert!(g.ids().any(|n| g.op(n) == Opcode::Load));
        for input in [0, 0x1234_5678, u64::MAX] {
            assert_eq!(
                Interpreter::new(&before).run(&[input], &mut |_: &str, _: &[u64]| Ok(0)),
                Interpreter::new(&g).run(&[input], &mut |_: &str, _: &[u64]| Ok(0))
            );
        }
    }
}

#[test]
fn integer_subranges_do_not_reinterpret_restricted_or_pointer_values() {
    for kind in 0..4 {
        let mut f = IntegerSlot::new(IntegerWidth::W64, false);
        let (shape, bytes) = match kind {
            0 => (TypeShape::Bool, 1),
            1 => (TypeShape::Char, 4),
            2 => (TypeShape::Float { bits: 64 }, 8),
            _ => (
                TypeShape::RawPointer {
                    mutability: kfrc_body::Mutability::Mut,
                    pointee: f.word,
                },
                8,
            ),
        };
        let ty = f.graph.oracle.intern(
            shape,
            Some(SizeAlign {
                size: bytes,
                align: 1,
            }),
        );
        f.store(0, f.argument);
        let value = f.load(0, ty);
        let mut g = f.finish(value);
        run(&mut g);
        verify_graph(&mut g).unwrap();
        assert!(g.ids().any(|n| g.op(n) == Opcode::Load));
    }
}

#[test]
fn subrange_writer_cannot_cross_a_new_storage_lifetime() {
    for marker in [Opcode::SlotLive, Opcode::SlotDead] {
        let mut f = IntegerSlot::new(IntegerWidth::W64, false);
        let byte = f.integer(IntegerWidth::W8, false);
        f.store(0, f.argument);
        fixed(
            &mut f.graph,
            &mut f.control,
            &mut f.effect,
            marker,
            &[f.slot],
            0,
            None,
        );
        let value = f.load(3, byte);
        let mut g = f.finish(value);
        run(&mut g);
        verify_graph(&mut g).unwrap();
        assert!(g.ids().any(|n| g.op(n) == Opcode::Load));
    }
}
