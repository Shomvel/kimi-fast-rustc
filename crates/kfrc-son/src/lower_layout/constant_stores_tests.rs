use super::*;
use crate::{Interpreter, Outcome, verify_graph};
use kfrc_body::{FunctionAbi, Mutability, PassMode, RecordedTypes, TyId};

struct Fixture {
    graph: FunctionGraph,
    byte: TyId,
    word: TyId,
    pointer: TyId,
    control: NodeId,
    effect: NodeId,
    slot: NodeId,
}

impl Fixture {
    fn new(size: u64) -> Self {
        let mut types = RecordedTypes::new();
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "constant_stores",
            FunctionAbi {
                args: vec![],
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
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
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
        graph.stack_slots.push(SizeAlign { size, align: 1 });
        let slot = graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(pointer),
        );
        Self {
            graph,
            byte,
            word,
            pointer,
            control,
            effect,
            slot,
        }
    }

    fn address(&mut self, offset: u64) -> NodeId {
        let offset = self.graph.add_constant(u128::from(offset), self.word);
        self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[self.slot, offset],
            0,
            ValueType::Rust(self.pointer),
        )
    }

    fn fixed(&mut self, op: Opcode, args: &[NodeId], result: Option<TyId>) -> Option<NodeId> {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(args);
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(result.map(ValueType::Rust));
        let node = self
            .graph
            .add_node(op, self.control, &inputs, 0, ValueType::Tuple(outputs));
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
        result.map(|ty| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[node],
                2,
                ValueType::Rust(ty),
            )
        })
    }

    fn store(&mut self, offset: u64, value: u8) {
        let value = self.graph.add_constant(u128::from(value), self.byte);
        let address = self.address(offset);
        self.fixed(Opcode::Store, &[address, value], None);
    }

    fn load(&mut self, offset: u64) -> NodeId {
        let address = self.address(offset);
        self.fixed(Opcode::Load, &[address], Some(self.byte))
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

fn outcome(graph: &FunctionGraph) -> Result<Outcome, crate::InterpretError> {
    Interpreter::new(graph).run(&[], &mut |_: &str, _: &[u64]| unreachable!())
}

#[test]
fn packed_unaligned_stores_preserve_every_byte_and_exact_tail() {
    for offset in 0..4 {
        for length in 2..32 {
            for seed in [0u8, 0x81, 0xff] {
                let mut t = Fixture::new(offset + length);
                let mut expected = 0u64;
                for index in 0..length {
                    let value = seed.wrapping_add((index as u8).wrapping_mul(37));
                    expected += u64::from(value) * (index + 1);
                    t.store(offset + index, value);
                }
                let mut sum = t.graph.add_constant(0, t.word);
                for index in 0..length {
                    let value = t.load(offset + index);
                    let value = t.graph.add_node(
                        Opcode::IntCast,
                        NodeId::NONE,
                        &[value],
                        0,
                        ValueType::Rust(t.word),
                    );
                    let factor = t.graph.add_constant(u128::from(index + 1), t.word);
                    let value = t.graph.add_node(
                        Opcode::Mul,
                        NodeId::NONE,
                        &[value, factor],
                        0,
                        ValueType::Rust(t.word),
                    );
                    sum = t.graph.add_node(
                        Opcode::Add,
                        NodeId::NONE,
                        &[sum, value],
                        0,
                        ValueType::Rust(t.word),
                    );
                }
                let mut graph = t.finish(sum);
                let before = outcome(&graph);
                assert_eq!(before, Ok(Outcome::Return(expected)));
                let removed = super::run(&mut graph);
                assert!(removed > 0);
                verify_graph(&mut graph).unwrap();
                assert_eq!(
                    outcome(&graph),
                    before,
                    "offset={offset} length={length} seed={seed}"
                );
                let stores: Vec<_> = graph
                    .ids()
                    .filter(|&n| graph.op(n) == Opcode::Store)
                    .collect();
                assert_eq!(stores.len(), length as usize - removed);
                let bytes: u64 = stores
                    .iter()
                    .map(|&n| {
                        graph
                            .oracle
                            .size_align(graph.rust_type(graph.input(n, 2)).unwrap())
                            .unwrap()
                            .size
                    })
                    .sum();
                assert_eq!(bytes, length);
            }
        }
    }
}

#[test]
fn an_intermediate_read_and_differing_or_repeated_addresses_keep_separate_stores() {
    for case in 0..3 {
        let mut t = Fixture::new(8);
        t.store(0, 17);
        let observed = if case == 0 { Some(t.load(0)) } else { None };
        t.store(
            if case == 1 {
                2
            } else if case == 2 {
                0
            } else {
                1
            },
            29,
        );
        let value = observed.unwrap_or_else(|| t.load(0));
        let mut graph = t.finish(value);
        let before = outcome(&graph);
        assert_eq!(super::run(&mut graph), 0);
        verify_graph(&mut graph).unwrap();
        assert_eq!(outcome(&graph), before);
    }
}

#[test]
fn an_extra_control_anchor_and_unknown_or_out_of_bounds_roots_block_packing() {
    for case in 0..3 {
        let mut t = Fixture::new(2);
        t.store(0, 17);
        if case == 0 {
            let value = t.graph.add_constant(3, t.word);
            t.graph.add_node(
                Opcode::Add,
                t.control,
                &[value, value],
                0,
                ValueType::Rust(t.word),
            );
        }
        t.store(1, 29);
        if case == 1 {
            t.graph.set_opcode(t.slot, Opcode::Undefined);
        } else if case == 2 {
            // Structurally valid L can still represent an invalid execution;
            // no packed store may extend beyond the known allocation extent.
            t.graph.stack_slots[0].size = 1;
        }
        let zero = t.graph.add_constant(0, t.word);
        let mut graph = t.finish(zero);
        assert_eq!(super::run(&mut graph), 0);
        verify_graph(&mut graph).unwrap();
    }
}
