use super::*;
use crate::{CallTarget, GraphLevel, Interpreter, Outcome, verify_graph};
use kfrc_body::{FunctionAbi, RecordedTypes, ScalarValidRange, ScalarValidity};

struct Fixture {
    graph: FunctionGraph,
    control: NodeId,
    effect: NodeId,
    source: NodeId,
    destination: NodeId,
    argument: NodeId,
    flag: NodeId,
    word: TyId,
    byte: TyId,
    index: TyId,
    pointer: TyId,
    const_pointer: TyId,
}

impl Fixture {
    fn new() -> Self {
        let mut types = RecordedTypes::new();
        let mut int = |width, size| {
            types.intern(
                TypeShape::Int {
                    width,
                    signed: false,
                },
                Some(SizeAlign { size, align: 1 }),
            )
        };
        let word = int(IntegerWidth::W64, 8);
        let byte = int(IntegerWidth::W8, 1);
        let index = int(IntegerWidth::Pointer, 8);
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let array = types.intern(
            TypeShape::Array {
                element: byte,
                length: 8,
            },
            Some(SizeAlign { size: 8, align: 1 }),
        );
        let storage = types.intern(
            TypeShape::PlaceAddress { pointee: array },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let const_pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "private_copy",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
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
                ValueType::Rust(boolean),
            ]),
        );
        graph.start = start;
        let mut projection =
            |index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[start], index, ty);
        let control = projection(0, ValueType::Control);
        let effect = projection(1, ValueType::Effect);
        let argument = projection(2, ValueType::Rust(word));
        let flag = projection(3, ValueType::Rust(boolean));
        graph.stack_slots = vec![
            SizeAlign { size: 16, align: 1 },
            SizeAlign { size: 8, align: 1 },
        ];
        let destination = graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(pointer),
        );
        let source = graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            1,
            ValueType::Rust(storage),
        );
        let mut f = Self {
            graph,
            control,
            effect,
            source,
            destination,
            argument,
            flag,
            word,
            byte,
            index,
            pointer,
            const_pointer,
        };
        for offset in 0..16 {
            let value = f.graph.add_constant(u128::from(0xa0 + offset), byte);
            f.store(destination, offset, value);
        }
        f.fixed(Opcode::SlotLive, &[source], 0, None);
        f
    }

    fn fixed(
        &mut self,
        op: Opcode,
        operands: &[NodeId],
        payload: u32,
        result: Option<TyId>,
    ) -> NodeId {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(operands);
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(result.map(ValueType::Rust));
        let node = self.graph.add_node(
            op,
            self.control,
            &inputs,
            payload,
            ValueType::Tuple(outputs),
        );
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            1,
            ValueType::Effect,
        );
        result.map_or(NodeId::NONE, |ty| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[node],
                2,
                ValueType::Rust(ty),
            )
        })
    }

    fn address(&mut self, base: NodeId, offset: u64, ty: TyId) -> NodeId {
        let offset = self.graph.add_constant(u128::from(offset), self.index);
        self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[base, offset],
            0,
            ValueType::Rust(ty),
        )
    }

    fn store(&mut self, base: NodeId, offset: u64, value: NodeId) {
        let address = self.address(base, offset, self.pointer);
        self.fixed(Opcode::Store, &[address, value], 0, None);
    }

    fn source_bytes(&mut self, count: u64) {
        // The observed v2 shape: normal masked shift, bswap, byte casts,
        // then distinct byte stores covering the temporary array.
        let zero = self.graph.add_constant(0, self.word);
        let shifted = self.graph.add_node(
            Opcode::ShiftLeft,
            NodeId::NONE,
            &[self.argument, zero],
            0,
            ValueType::Rust(self.word),
        );
        let swapped = self.graph.add_node(
            Opcode::ByteSwap,
            NodeId::NONE,
            &[shifted],
            0,
            ValueType::Rust(self.word),
        );
        for offset in 0..count {
            let shift = self.graph.add_constant(u128::from(offset * 8), self.word);
            let value = self.graph.add_node(
                Opcode::ShiftRightLogical,
                NodeId::NONE,
                &[swapped, shift],
                0,
                ValueType::Rust(self.word),
            );
            let value = self.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[value],
                0,
                ValueType::Rust(self.byte),
            );
            self.store(self.source, offset, value);
        }
    }

    fn call(&mut self, symbol: &str, operands: &[NodeId], result: TyId) -> NodeId {
        let target = self.graph.add_callee(CallTarget {
            symbol: symbol.into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct; operands.len()],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        self.fixed(Opcode::Call, operands, target, Some(result))
    }

    fn copy_to(&mut self, destination: NodeId, offset: u64, bytes: u64) -> NodeId {
        let source = self.address(self.source, offset, self.const_pointer);
        let count = self.graph.add_constant(u128::from(bytes), self.index);
        self.call("memcpy", &[destination, source, count], self.pointer)
    }

    fn copy(&mut self, bytes: u64) -> NodeId {
        let destination = self.address(self.destination, 1, self.pointer);
        self.copy_to(destination, 0, bytes)
    }

    fn merge(&mut self, different: bool) {
        let branch = self.graph.add_node(
            Opcode::If,
            self.control,
            &[self.flag],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let incoming = self.effect;
        let no = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            1,
            ValueType::Control,
        );
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            0,
            ValueType::Control,
        );
        if different {
            let value = self.graph.add_constant(0x55, self.byte);
            self.store(self.source, 2, value);
        }
        let yes_effect = self.effect;
        let yes = self
            .graph
            .add_node(Opcode::Jump, self.control, &[], 0, ValueType::Control);
        let no = self
            .graph
            .add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
        self.control = self.graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[yes, no],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[self.control, yes_effect, incoming],
            0,
            ValueType::Effect,
        );
    }

    fn finish(mut self, result: NodeId) -> FunctionGraph {
        self.fixed(Opcode::SlotDead, &[self.source], 0, None);
        for offset in 0..16 {
            let address = self.address(self.destination, offset, self.pointer);
            let byte = self.fixed(Opcode::Load, &[address], 0, Some(self.byte));
            let offset = self.graph.add_constant(u128::from(offset), self.index);
            self.call("observe_byte", &[offset, byte], self.word);
        }
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, result],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
        self.graph
    }
}

fn has_copy(g: &FunctionGraph) -> bool {
    g.ids()
        .any(|n| g.op(n) == Opcode::Call && g.callees[g.payload(n) as usize].symbol == "memcpy")
}

fn observe(g: &FunctionGraph, input: u64, flag: u64) -> (Outcome, [u8; 16]) {
    let mut bytes = [0; 16];
    let mut seen = [false; 16];
    let outcome = Interpreter::new(g)
        .run(&[input, flag], &mut |name: &str, args: &[u64]| {
            assert_eq!(name, "observe_byte");
            let offset = args[0] as usize;
            assert!(!seen[offset]);
            seen[offset] = true;
            bytes[offset] = args[1] as u8;
            Ok(0)
        })
        .unwrap();
    assert!(seen.into_iter().all(|seen| seen));
    (outcome, bytes)
}

#[test]
fn exact_six_and_three_byte_copies_preserve_odd_destination_and_full_suffix() {
    for count in 1..=8 {
        let mut f = Fixture::new();
        f.source_bytes(8);
        let result = f.copy(count);
        let before = f.finish(result);
        let mut after = before.clone();
        super::super::run(&mut after);
        verify_graph(&mut after).unwrap();
        assert!(!has_copy(&after));
        assert_eq!(
            after
                .ids()
                .filter(|&n| after.op(n) == Opcode::Store)
                .count(),
            16 + count.count_ones() as usize
        );
        assert_eq!(
            after
                .ids()
                .filter(|&n| after.op(n) == Opcode::StackSlot)
                .count(),
            1
        );
        for input in [0, 1, 0x817f_ff00_55aa_fe01, u64::MAX] {
            let mut expected = std::array::from_fn(|i| 0xa0 + i as u8);
            expected[1..1 + count as usize].copy_from_slice(&input.to_be_bytes()[..count as usize]);
            let result = observe(&after, input, 0);
            assert_eq!(result, observe(&before, input, 0));
            assert_eq!(result, (Outcome::Return(0x1001), expected));
        }
    }
}

#[test]
fn contained_integer_writer_snapshots_exact_source_subranges() {
    for count in 1..=8 {
        for offset in 0..=8 - count {
            let mut f = Fixture::new();
            f.store(f.source, 0, f.argument);
            let destination = f.address(f.destination, 1, f.pointer);
            let result = f.copy_to(destination, offset, count);
            let before = f.finish(result);
            let mut after = before.clone();
            super::super::run(&mut after);
            verify_graph(&mut after).unwrap();
            assert!(!has_copy(&after));
            let input = 0x817f_ff00_55aa_fe01u64;
            let mut expected = std::array::from_fn(|i| 0xa0 + i as u8);
            expected[1..1 + count as usize]
                .copy_from_slice(&input.to_le_bytes()[offset as usize..(offset + count) as usize]);
            assert_eq!(observe(&after, input, 0), observe(&before, input, 0));
            assert_eq!(observe(&after, input, 0).1, expected);
        }
    }
}

#[test]
fn unrelated_and_reordered_bytes_keep_their_original_values() {
    for reorder in [false, true] {
        let mut f = Fixture::new();
        let mask = f.graph.add_constant(0xa55a_00ff_817e_23dc, f.word);
        let other = f.graph.add_node(
            Opcode::BitXor,
            NodeId::NONE,
            &[f.argument, mask],
            0,
            ValueType::Rust(f.word),
        );
        for offset in 0..8 {
            let source = if !reorder && offset % 2 == 1 {
                other
            } else {
                f.argument
            };
            let byte = if reorder { 7 - offset } else { offset };
            let shift = f.graph.add_constant(byte * 8, f.word);
            let value = f.graph.add_node(
                Opcode::ShiftRightLogical,
                NodeId::NONE,
                &[source, shift],
                0,
                ValueType::Rust(f.word),
            );
            let value = f.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[value],
                0,
                ValueType::Rust(f.byte),
            );
            f.store(f.source, offset as u64, value);
        }
        let result = f.copy(8);
        let before = f.finish(result);
        let mut after = before.clone();
        super::super::run(&mut after);
        verify_graph(&mut after).unwrap();
        assert!(!has_copy(&after));
        assert_eq!(
            after
                .ids()
                .filter(|&n| after.op(n) == Opcode::Store)
                .count(),
            24
        );
        for input in [0, 1, 0x817f_ff00_55aa_fe01, u64::MAX] {
            assert_eq!(observe(&after, input, 0), observe(&before, input, 0));
        }
    }
}

#[test]
fn byte_origins_use_the_shifted_operands_width_for_masking() {
    let mut f = Fixture::new();
    let half = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W16,
            signed: false,
        },
        Some(SizeAlign { size: 2, align: 1 }),
    );
    let value = f.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[f.argument],
        0,
        ValueType::Rust(half),
    );
    for offset in 0..2 {
        // 16 and 24 mask to 0 and 8 for u16, regardless of the u64 RHS.
        let shift = f.graph.add_constant(16 + offset * 8, f.word);
        let shifted = f.graph.add_node(
            Opcode::ShiftRightLogical,
            NodeId::NONE,
            &[value, shift],
            0,
            ValueType::Rust(half),
        );
        let byte = f.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[shifted],
            0,
            ValueType::Rust(f.byte),
        );
        f.store(f.source, offset as u64, byte);
    }
    let result = f.copy(2);
    let before = f.finish(result);
    let mut after = before.clone();
    super::super::run(&mut after);
    verify_graph(&mut after).unwrap();
    assert!(!has_copy(&after));
    assert_eq!(
        after
            .ids()
            .filter(|&n| after.op(n) == Opcode::Store)
            .count(),
        17
    );
    for input in [0, 1, 0x817f_ff00_55aa_fe01, u64::MAX] {
        let (outcome, bytes) = observe(&after, input, 0);
        assert_eq!((outcome, bytes), observe(&before, input, 0));
        assert_eq!(&bytes[1..3], &input.to_le_bytes()[..2]);
    }
}

#[test]
fn chunk_types_cannot_reuse_restricted_integer_validity() {
    let mut f = Fixture::new();
    let quarter = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W32,
            signed: false,
        },
        Some(SizeAlign { size: 4, align: 1 }),
    );
    f.graph.oracle.record_scalar_validity(
        quarter,
        ScalarValidity::Scalar(ScalarValidRange::new(32, 1, u128::from(u32::MAX)).unwrap()),
    );
    f.source_bytes(8);
    let result = f.copy(6);
    let before = f.finish(result);
    let mut after = before.clone();
    super::super::run(&mut after);
    verify_graph(&mut after).unwrap();
    assert!(!has_copy(&after));
    assert!(
        after
            .ids()
            .filter(|&n| after.op(n) == Opcode::Store)
            .all(|n| after.rust_type(after.input(n, 2)) != Some(quarter))
    );
    for input in [0, 1, 0x817f_ff00_55aa_fe01, u64::MAX] {
        assert_eq!(observe(&after, input, 0), observe(&before, input, 0));
    }
}

#[test]
fn effect_joins_require_the_same_writer_for_every_copied_byte() {
    for different in [false, true] {
        let mut f = Fixture::new();
        f.source_bytes(8);
        f.merge(different);
        let result = f.copy(6);
        let before = f.finish(result);
        let mut after = before.clone();
        super::super::run(&mut after);
        verify_graph(&mut after).unwrap();
        assert_eq!(has_copy(&after), different);
        for flag in [0, 1] {
            assert_eq!(
                observe(&after, 0x1234_5678_9abc_def0, flag),
                observe(&before, 0x1234_5678_9abc_def0, flag)
            );
        }
    }
}

#[test]
fn source_lifetime_and_incomplete_initialization_retain_the_copy() {
    for marker in [None, Some(Opcode::SlotLive), Some(Opcode::SlotDead)] {
        let mut f = Fixture::new();
        f.source_bytes(if marker.is_some() { 8 } else { 5 });
        if let Some(marker) = marker {
            f.fixed(marker, &[f.source], 0, None);
        }
        let result = f.copy(6);
        let mut g = f.finish(result);
        super::super::run(&mut g);
        verify_graph(&mut g).unwrap();
        assert!(has_copy(&g));
    }
}

#[test]
fn partial_source_destination_overlap_and_address_escapes_retain_the_copy() {
    for kind in 0..4 {
        let mut f = Fixture::new();
        f.source_bytes(8);
        if kind == 1 {
            let source = f.address(f.source, 0, f.const_pointer);
            f.call("escape", &[source], f.word);
        } else if kind == 2 {
            f.copy(3);
        } else if kind == 3 {
            let dynamic = f.graph.add_node(
                Opcode::PtrOffset,
                NodeId::NONE,
                &[f.source, f.argument],
                0,
                ValueType::Rust(f.const_pointer),
            );
            f.fixed(Opcode::Load, &[dynamic], 0, Some(f.byte));
        }
        let result = if kind == 0 {
            let overlapping = f.address(f.source, 1, f.pointer);
            f.copy_to(overlapping, 0, 6)
        } else {
            f.copy(6)
        };
        let mut g = f.finish(result);
        super::super::run(&mut g);
        verify_graph(&mut g).unwrap();
        assert!(has_copy(&g));
    }
}

#[test]
fn unknown_poison_and_restricted_integer_representations_retain_the_copy() {
    for kind in 0..4 {
        let mut f = Fixture::new();
        let value = match kind {
            0 => f.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.word),
            ),
            1 => {
                let shift = f.graph.add_constant(0, f.word);
                f.graph.add_node(
                    Opcode::ShiftRightLogical,
                    NodeId::NONE,
                    &[f.argument, shift],
                    crate::ArithmeticFlags::EXACT,
                    ValueType::Rust(f.word),
                )
            }
            2 => {
                f.graph.oracle.record_scalar_validity(
                    f.word,
                    ScalarValidity::Scalar(
                        ScalarValidRange::new(64, 1, u128::from(u64::MAX)).unwrap(),
                    ),
                );
                f.argument
            }
            _ => {
                let boolean = f.graph.rust_type(f.flag).unwrap();
                let value = f.graph.add_constant(1, boolean);
                f.graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[value],
                    0,
                    ValueType::Rust(f.word),
                )
            }
        };
        f.store(f.source, 0, value);
        let result = f.copy(3);
        let mut g = f.finish(result);
        super::super::run(&mut g);
        verify_graph(&mut g).unwrap();
        assert!(has_copy(&g));
    }
}

#[test]
fn memcpy_name_alone_does_not_authorize_other_abi_count_or_result_shapes() {
    for kind in 0..7 {
        let mut f = Fixture::new();
        f.source_bytes(8);
        let result = f.copy(if kind == 0 {
            0
        } else if kind == 1 {
            9
        } else {
            6
        });
        let call = f.graph.input(result, 0);
        let target = f.graph.payload(call) as usize;
        match kind {
            2 => f.graph.callees[target].abi.c_variadic = true,
            3 => f.graph.callees[target].abi.args[0] = PassMode::Indirect,
            4 => f.graph.callees[target].abi.ret = PassMode::Ignore,
            5 => f.graph.set_input(call, 3, f.argument),
            6 => {
                let mut results = match f.graph.value_type(call).clone() {
                    ValueType::Tuple(results) => results,
                    _ => unreachable!(),
                };
                results[2] = ValueType::Rust(f.word);
                f.graph.set_value_type(call, ValueType::Tuple(results));
                f.graph.set_value_type(result, ValueType::Rust(f.word));
            }
            _ => {}
        }
        run(&mut f.graph);
        assert!(has_copy(&f.graph));
    }
}

#[test]
fn cyclic_effect_writer_search_retains_the_copy() {
    let mut f = Fixture::new();
    f.source_bytes(8);
    let entry = f
        .graph
        .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
    let region = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, f.effect, f.effect],
        0,
        ValueType::Effect,
    );
    f.graph.set_input(effect, 2, effect);
    let branch = f.graph.add_node(
        Opcode::If,
        region,
        &[f.flag],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let yes = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        0,
        ValueType::Control,
    );
    let back = f
        .graph
        .add_node(Opcode::Jump, yes, &[], 0, ValueType::Control);
    f.graph.set_inputs(region, &[entry, back]);
    f.control = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        1,
        ValueType::Control,
    );
    f.effect = effect;
    let result = f.copy(6);
    let mut g = f.finish(result);
    super::super::run(&mut g);
    verify_graph(&mut g).unwrap();
    assert!(has_copy(&g));
}
