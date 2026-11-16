use super::*;
use crate::{ComparePredicate as P, Interpreter, Outcome, verify_graph};
use kfrc_body::{FunctionAbi, PassMode, RecordedTypes};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    pointer: TyId,
    control: NodeId,
    effect: NodeId,
    args: [NodeId; 2],
    slot: NodeId,
}

fn project(graph: &mut FunctionGraph, node: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[node], slot, ty)
}

impl Fixture {
    fn new() -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let pointer = types.intern(TypeShape::PlaceAddress { pointee: word }, None);
        let mut graph = FunctionGraph::new(
            "lane_state",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Layout;
        graph.start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
                ValueType::Rust(word),
            ]),
        );
        let start = graph.start;
        let control = project(&mut graph, start, 0, ValueType::Control);
        let effect = project(&mut graph, start, 1, ValueType::Effect);
        let args = [2, 3].map(|slot| project(&mut graph, start, slot, ValueType::Rust(word)));
        graph.stack_slots.push(SizeAlign { size: 16, align: 8 });
        let slot = graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(pointer),
        );
        Self {
            graph,
            word,
            boolean,
            pointer,
            control,
            effect,
            args,
            slot,
        }
    }

    fn constant(&mut self, value: u64) -> NodeId {
        self.graph.add_constant(u128::from(value), self.word)
    }

    fn address(&mut self, offset: u64) -> NodeId {
        let offset = self.constant(offset);
        self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[self.slot, offset],
            0,
            ValueType::Rust(self.pointer),
        )
    }

    fn fixed(&mut self, op: Opcode, operands: &[NodeId], result: Option<TyId>) -> Option<NodeId> {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(operands);
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(result.map(ValueType::Rust));
        let node = self
            .graph
            .add_node(op, self.control, &inputs, 0, ValueType::Tuple(outputs));
        self.control = project(&mut self.graph, node, 0, ValueType::Control);
        self.effect = project(&mut self.graph, node, 1, ValueType::Effect);
        result.map(|ty| project(&mut self.graph, node, 2, ValueType::Rust(ty)))
    }

    fn store(&mut self, offset: u64, value: NodeId) {
        let address = self.address(offset);
        self.fixed(Opcode::Store, &[address, value], None);
    }

    fn load(&mut self, offset: u64) -> NodeId {
        let address = self.address(offset);
        self.fixed(Opcode::Load, &[address], Some(self.word))
            .unwrap()
    }

    fn branch(&mut self, condition: NodeId) -> [NodeId; 2] {
        let node = self.graph.add_node(
            Opcode::If,
            self.control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        [0, 1].map(|slot| project(&mut self.graph, node, slot, ValueType::Control))
    }

    fn compare(&mut self, predicate: P, a: NodeId, b: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[a, b],
            predicate as u32,
            ValueType::Rust(self.boolean),
        )
    }

    fn jump(&mut self) -> NodeId {
        self.graph
            .add_node(Opcode::Jump, self.control, &[], 0, ValueType::Control)
    }

    fn join(&mut self, incoming: &[(NodeId, NodeId)]) {
        let jumps: Vec<_> = incoming.iter().map(|&(jump, _)| jump).collect();
        self.control =
            self.graph
                .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
        let mut effects = vec![self.control];
        effects.extend(incoming.iter().map(|&(_, effect)| effect));
        self.effect =
            self.graph
                .add_node(Opcode::Phi, NodeId::NONE, &effects, 0, ValueType::Effect);
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

fn execute(graph: &FunctionGraph, a: u64, b: u64) -> Result<Outcome, crate::InterpretError> {
    Interpreter::new(graph).run(&[a, b], &mut |_: &str, _: &[u64]| unreachable!())
}

#[test]
fn different_predecessor_fields_become_scalar_phis_before_threading() {
    let mut f = Fixture::new();
    let zero = f.constant(0);
    let one = f.constant(1);
    let small = f.constant(8);
    let large = f.constant(32);
    let condition = f.compare(P::NotEqual, f.args[0], zero);
    let [yes, no] = f.branch(condition);
    let effect = f.effect;
    let mut incoming = vec![];
    for (control, tag, length) in [(yes, one, large), (no, zero, small)] {
        f.control = control;
        f.effect = effect;
        f.store(0, tag);
        f.store(8, length);
        incoming.push((f.jump(), f.effect));
    }
    f.join(&incoming);
    let tag = f.load(0);
    let length = f.load(8);
    let product = f.graph.add_node(
        Opcode::Mul,
        NodeId::NONE,
        &[tag, length],
        0,
        ValueType::Rust(f.word),
    );
    let mut after = f.finish(product);
    let before = after.clone();
    assert_eq!(run(&mut after), 1);
    verify_graph(&mut after).unwrap();
    assert!(
        !after
            .ids()
            .any(|n| matches!(after.op(n), Opcode::Load | Opcode::Store))
    );
    assert!(
        after.ids().any(
            |n| after.op(n) == Opcode::Phi && matches!(after.value_type(n), ValueType::Rust(_))
        )
    );
    for selector in [0, 1, 2, u64::MAX] {
        assert_eq!(execute(&before, selector, 0), execute(&after, selector, 0));
        assert_eq!(
            execute(&after, selector, 0),
            Ok(Outcome::Return(if selector == 0 { 0 } else { 32 }))
        );
    }
}

#[test]
fn initialized_loop_keeps_snapshots_and_backedge_writes_in_order() {
    let mut f = Fixture::new();
    let zero = f.constant(0);
    let one = f.constant(1);
    f.store(0, zero);
    let entry = f.jump();
    let entry_effect = f.effect;
    let region = f
        .graph
        .add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    f.graph.set_input(region, 0, entry);
    let effect = f
        .graph
        .add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Effect);
    f.graph.set_input(effect, 0, region);
    f.graph.set_input(effect, 1, entry_effect);
    f.control = region;
    f.effect = effect;
    let old = f.load(0);
    let condition = f.compare(P::UnsignedLess, old, f.args[0]);
    let [body, exit] = f.branch(condition);
    let loaded_effect = f.effect;
    f.control = body;
    let next = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[old, one],
        0,
        ValueType::Rust(f.word),
    );
    f.store(0, next);
    let backedge = f.jump();
    f.graph.set_input(region, 1, backedge);
    f.graph.set_input(effect, 2, f.effect);
    f.control = exit;
    f.effect = loaded_effect;
    let result = f.load(0);
    let mut after = f.finish(result);
    let before = after.clone();
    assert_eq!(run(&mut after), 1);
    verify_graph(&mut after).unwrap();
    assert!(
        !after
            .ids()
            .any(|n| matches!(after.op(n), Opcode::Load | Opcode::Store))
    );
    for count in [0, 1, 2, 17, 255] {
        assert_eq!(execute(&before, count, 0), execute(&after, count, 0));
        assert_eq!(execute(&after, count, 0), Ok(Outcome::Return(count)));
    }
}

#[test]
fn invariant_lane_crosses_nested_joins_and_loop_backedges_as_one_value() {
    let mut f = Fixture::new();
    let zero = f.constant(0);
    let one = f.constant(1);
    let stride = f.constant(32);
    f.store(0, zero);
    f.store(8, stride);
    let entry = f.jump();
    let entry_effect = f.effect;
    let header = f
        .graph
        .add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    f.graph.set_input(header, 0, entry);
    let effect = f
        .graph
        .add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Effect);
    f.graph.set_input(effect, 0, header);
    f.graph.set_input(effect, 1, entry_effect);
    f.control = header;
    f.effect = effect;
    let old = f.load(0);
    let condition = f.compare(P::UnsignedLess, old, f.args[0]);
    let [body, exit] = f.branch(condition);
    let exit_effect = f.effect;
    f.control = body;
    // Each join introduces another candidate state Phi for the unchanged
    // stride. Their backedge dependencies must not obscure the initial 32.
    for _ in 0..10 {
        let condition = f.compare(P::NotEqual, f.args[1], zero);
        let [left, right] = f.branch(condition);
        let effect = f.effect;
        let mut incoming = Vec::new();
        for control in [left, right] {
            f.control = control;
            incoming.push((f.jump(), effect));
        }
        f.join(&incoming);
    }
    let length = f.load(8);
    let condition = f.compare(P::UnsignedLessEqual, stride, length);
    let [yes, no] = f.branch(condition);
    f.graph
        .add_node(Opcode::Unreachable, no, &[], 0, ValueType::Control);
    f.control = yes;
    let next = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[old, one],
        0,
        ValueType::Rust(f.word),
    );
    f.store(0, next);
    let latch = f.jump();
    f.graph.set_input(header, 1, latch);
    f.graph.set_input(effect, 2, f.effect);
    f.control = exit;
    f.effect = exit_effect;
    let result = f.load(0);
    let mut after = f.finish(result);
    let before = after.clone();
    assert_eq!(run(&mut after), 1);
    verify_graph(&mut after).unwrap();
    let guard = after
        .ids()
        .find(|&node| {
            after.op(node) == Opcode::Compare && after.payload(node) == P::UnsignedLessEqual as u32
        })
        .unwrap();
    for &input in after.inputs(guard) {
        assert_eq!(after.op(input), Opcode::Const);
        assert_eq!(after.constant_value(input), 32);
    }
    cleanup::run(&mut after).unwrap();
    verify_graph(&mut after).unwrap();
    assert!(
        !after
            .ids()
            .any(|node| after.op(node) == Opcode::Unreachable)
    );
    for count in [0, 1, 2, 17, 255] {
        for selector in [0, 1, u64::MAX] {
            assert_eq!(
                execute(&before, count, selector),
                execute(&after, count, selector)
            );
            assert_eq!(execute(&after, count, selector), Ok(Outcome::Return(count)));
        }
    }
}

#[test]
fn dead_partial_overlap_and_escaped_storage_stay_memory() {
    for reason in 1..4 {
        let mut f = Fixture::new();
        let value = f.args[0];
        f.store(0, value);
        match reason {
            1 => {
                f.fixed(Opcode::SlotDead, &[f.slot], None);
            }
            2 => {
                let byte = f.graph.oracle.intern(
                    TypeShape::Int {
                        width: IntegerWidth::W8,
                        signed: false,
                    },
                    Some(SizeAlign { size: 1, align: 1 }),
                );
                let value = f.graph.add_constant(19, byte);
                let address = f.address(1);
                f.fixed(Opcode::Store, &[address, value], None);
            }
            3 => {
                let pointer = f.graph.oracle.intern(
                    TypeShape::RawPointer {
                        mutability: kfrc_body::Mutability::Mut,
                        pointee: f.word,
                    },
                    Some(SizeAlign { size: 8, align: 8 }),
                );
                let zero = f.constant(0);
                let address = f.graph.add_node(
                    Opcode::PtrOffset,
                    NodeId::NONE,
                    &[f.slot, zero],
                    0,
                    ValueType::Rust(pointer),
                );
                f.store(8, address);
            }
            _ => {}
        }
        let result = f.load(0);
        let mut after = f.finish(result);
        let before = after.clone();
        assert_eq!(run(&mut after), 0);
        verify_graph(&mut after).unwrap();
        assert!(after.ids().any(|n| after.op(n) == Opcode::Load));
        for value in [0, 1, 0x807f00ff, u64::MAX] {
            // Error node IDs are unchanged when promotion declines the slot.
            assert_eq!(execute(&before, value, 0), execute(&after, value, 0));
        }
    }
}

#[test]
fn missing_initialization_stays_poison_and_new_lifetime_discards_the_old_value() {
    for restart in [false, true] {
        let mut f = Fixture::new();
        if restart {
            let value = f.args[0];
            f.store(0, value);
            f.fixed(Opcode::SlotLive, &[f.slot], None);
        }
        let value = f.load(0);
        let mut after = f.finish(value);
        let before = after.clone();
        assert_eq!(run(&mut after), 1);
        verify_graph(&mut after).unwrap();
        assert!(!after.ids().any(|n| after.op(n) == Opcode::Load));
        for value in [0, 37, u64::MAX] {
            for graph in [&before, &after] {
                assert!(matches!(
                    execute(graph, value, 0),
                    Err(crate::InterpretError::PoisonObserved(_))
                ));
            }
        }
    }
}

#[test]
fn phi_simplification_preserves_an_uninitialized_predecessor() {
    let mut f = Fixture::new();
    let zero = f.constant(0);
    let value = f.constant(32);
    let condition = f.compare(P::NotEqual, f.args[0], zero);
    let [initialized, unwritten] = f.branch(condition);
    let effect = f.effect;
    f.control = initialized;
    f.store(0, value);
    let written = (f.jump(), f.effect);
    f.control = unwritten;
    f.effect = effect;
    let unwritten = (f.jump(), f.effect);
    f.join(&[written, unwritten]);
    let value = f.load(0);
    let word = f.word;
    let mut after = f.finish(value);
    let before = after.clone();
    assert_eq!(run(&mut after), 1);
    verify_graph(&mut after).unwrap();
    assert!(
        after.ids().any(|node| after.op(node) == Opcode::Phi
            && after.value_type(node) == &ValueType::Rust(word))
    );
    for graph in [&before, &after] {
        assert!(matches!(
            execute(graph, 0, 0),
            Err(crate::InterpretError::PoisonObserved(_))
        ));
        assert_eq!(execute(graph, 1, 0), Ok(Outcome::Return(32)));
    }
}
