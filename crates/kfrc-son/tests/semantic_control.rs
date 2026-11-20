use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};
use kfrc_son::{
    CallTarget, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode,
    SemanticOutcome, Value, ValueType, optimize_specialized_graph, verify_graph,
};

struct TestGraph {
    graph: FunctionGraph,
    integer: TyId,
    boolean: TyId,
    control: NodeId,
    effect: NodeId,
}

impl TestGraph {
    fn new() -> Self {
        let mut types = RecordedTypes::new();
        let integer = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let mut graph = FunctionGraph::new(
            "control",
            FunctionAbi {
                args: Vec::new(),
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
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
        Self {
            graph,
            integer,
            boolean,
            control,
            effect,
        }
    }

    fn branch(&mut self, condition: NodeId) -> (NodeId, NodeId) {
        let branch = self.graph.add_node(
            Opcode::If,
            self.control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        let yes = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            0,
            ValueType::Control,
        );
        let no = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            1,
            ValueType::Control,
        );
        (yes, no)
    }

    fn return_value(&mut self, control: NodeId, effect: NodeId, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            control,
            &[effect, value],
            0,
            ValueType::Control,
        );
    }

    fn wrap_control(&mut self, mut control: NodeId, count: usize) -> NodeId {
        for _ in 0..count {
            let jump = self
                .graph
                .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
            control =
                self.graph
                    .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
        }
        control
    }

    fn call(&mut self, control: NodeId, symbol: &str) -> (NodeId, NodeId, NodeId) {
        let target = self.graph.add_callee(CallTarget {
            symbol: symbol.into(),
            abi: FunctionAbi {
                args: Vec::new(),
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        let call = self.graph.add_node(
            Opcode::Call,
            control,
            &[self.effect],
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(self.integer),
            ]),
        );
        let control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            0,
            ValueType::Control,
        );
        let effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            1,
            ValueType::Effect,
        );
        let value = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            2,
            ValueType::Rust(self.integer),
        );
        (control, effect, value)
    }
}

fn run(graph: &FunctionGraph) -> (Result<SemanticOutcome, InterpretError>, Vec<String>) {
    let mut trace = Vec::new();
    let result = Interpreter::new(graph).run_values(&[], &mut |name: &str, _: &[Value]| {
        trace.push(name.to_owned());
        Ok(Value::Scalar(if name == "left" { 37 } else { 93 }))
    });
    (result, trace)
}

#[test]
fn constant_branch_preserves_call_effects_and_all_merge_values() {
    for take_left in [false, true] {
        let mut test = TestGraph::new();
        let condition = test.graph.add_constant(u128::from(take_left), test.boolean);
        let (left, right) = test.branch(condition);
        let (left_ctl, left_effect, left_value) = test.call(left, "left");
        let (right_ctl, right_effect, right_value) = test.call(right, "right");
        let left_jump = test
            .graph
            .add_node(Opcode::Jump, left_ctl, &[], 0, ValueType::Control);
        let right_jump = test
            .graph
            .add_node(Opcode::Jump, right_ctl, &[], 0, ValueType::Control);
        let region = test.graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[left_jump, right_jump],
            0,
            ValueType::Control,
        );
        let effect = test.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, left_effect, right_effect],
            0,
            ValueType::Effect,
        );
        let value = test.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, left_value, right_value],
            0,
            ValueType::Rust(test.integer),
        );
        let ten = test.graph.add_constant(10, test.integer);
        let hundred = test.graph.add_constant(100, test.integer);
        let offset = test.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, ten, hundred],
            0,
            ValueType::Rust(test.integer),
        );
        let sum = test.graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[value, offset],
            0,
            ValueType::Rust(test.integer),
        );
        test.return_value(region, effect, sum);
        verify_graph(&mut test.graph).unwrap();
        let before = run(&test.graph);
        assert_eq!(
            before.0,
            Ok(SemanticOutcome::Return(Value::Scalar(if take_left {
                47
            } else {
                193
            })))
        );
        assert_eq!(before.1, [if take_left { "left" } else { "right" }]);
        optimize_specialized_graph(&mut test.graph).unwrap();
        verify_graph(&mut test.graph).unwrap();
        assert_eq!(run(&test.graph), before);
        assert!(
            !test
                .graph
                .ids()
                .any(|node| matches!(test.graph.op(node), Opcode::If | Opcode::Phi))
        );
        assert_eq!(
            test.graph
                .ids()
                .filter(|node| test.graph.op(*node) == Opcode::Call)
                .count(),
            1
        );
    }
}

#[test]
fn unreachable_loop_and_its_cyclic_phi_are_removed() {
    let mut test = TestGraph::new();
    let yes = test.graph.add_constant(1, test.boolean);
    let (exit, loop_entry) = test.branch(yes);
    let answer = test.graph.add_constant(42, test.integer);
    test.return_value(exit, test.effect, answer);
    let initial = test
        .graph
        .add_node(Opcode::Jump, loop_entry, &[], 0, ValueType::Control);
    let region =
        test.graph
            .add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    test.graph.set_input(region, 0, initial);
    let phi = test.graph.add_growable_node(
        Opcode::Phi,
        NodeId::NONE,
        3,
        0,
        ValueType::Rust(test.integer),
    );
    test.graph.set_input(phi, 0, region);
    let zero = test.graph.add_constant(0, test.integer);
    let one = test.graph.add_constant(1, test.integer);
    test.graph.set_input(phi, 1, zero);
    let next = test.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[phi, one],
        0,
        ValueType::Rust(test.integer),
    );
    test.graph.set_input(phi, 2, next);
    let back = test
        .graph
        .add_node(Opcode::Jump, region, &[], 0, ValueType::Control);
    test.graph.set_input(region, 1, back);
    verify_graph(&mut test.graph).unwrap();
    let before = run(&test.graph);
    optimize_specialized_graph(&mut test.graph).unwrap();
    verify_graph(&mut test.graph).unwrap();
    assert_eq!(run(&test.graph), before);
    assert_eq!(before.0, Ok(SemanticOutcome::Return(Value::Scalar(42))));
    assert!(!test.graph.ids().any(|node| matches!(
        test.graph.op(node),
        Opcode::Region | Opcode::Phi | Opcode::Jump
    )));
}

#[test]
fn unused_traps_on_the_live_path_survive_control_and_compaction() {
    let mut test = TestGraph::new();
    let yes = test.graph.add_constant(1, test.boolean);
    let (taken, other) = test.branch(yes);
    let one = test.graph.add_constant(1, test.integer);
    let zero = test.graph.add_constant(0, test.integer);
    test.graph.add_node(
        Opcode::DivUnsigned,
        taken,
        &[one, zero],
        0,
        ValueType::Rust(test.integer),
    );
    test.return_value(taken, test.effect, one);
    test.return_value(other, test.effect, zero);
    verify_graph(&mut test.graph).unwrap();
    assert!(matches!(
        run(&test.graph).0,
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    optimize_specialized_graph(&mut test.graph).unwrap();
    verify_graph(&mut test.graph).unwrap();
    assert!(matches!(
        run(&test.graph).0,
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert!(
        test.graph
            .ids()
            .any(|node| test.graph.op(node) == Opcode::DivUnsigned)
    );
}

#[test]
fn an_untaken_trap_is_removed_without_becoming_observable() {
    let mut test = TestGraph::new();
    let yes = test.graph.add_constant(1, test.boolean);
    let (taken, other) = test.branch(yes);
    let one = test.graph.add_constant(1, test.integer);
    let zero = test.graph.add_constant(0, test.integer);
    test.graph.add_node(
        Opcode::DivUnsigned,
        other,
        &[one, zero],
        0,
        ValueType::Rust(test.integer),
    );
    test.return_value(taken, test.effect, one);
    test.return_value(other, test.effect, zero);
    verify_graph(&mut test.graph).unwrap();
    let before = run(&test.graph);
    optimize_specialized_graph(&mut test.graph).unwrap();
    verify_graph(&mut test.graph).unwrap();
    assert_eq!(run(&test.graph), before);
    assert_eq!(before.0, Ok(SemanticOutcome::Return(Value::Scalar(1))));
    assert!(
        !test
            .graph
            .ids()
            .any(|node| test.graph.op(node) == Opcode::DivUnsigned)
    );
}

#[test]
fn poison_condition_is_not_folded_to_an_arbitrary_path() {
    let mut test = TestGraph::new();
    let poison = test.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(test.boolean),
    );
    let (left, right) = test.branch(poison);
    let one = test.graph.add_constant(1, test.integer);
    let zero = test.graph.add_constant(0, test.integer);
    test.return_value(left, test.effect, one);
    test.return_value(right, test.effect, zero);
    verify_graph(&mut test.graph).unwrap();
    assert!(matches!(
        run(&test.graph).0,
        Err(InterpretError::PoisonObserved(_))
    ));
    optimize_specialized_graph(&mut test.graph).unwrap();
    verify_graph(&mut test.graph).unwrap();
    assert!(matches!(
        run(&test.graph).0,
        Err(InterpretError::PoisonObserved(_))
    ));
    assert!(
        test.graph
            .ids()
            .any(|node| test.graph.op(node) == Opcode::If)
    );
}

#[test]
fn boolean_diamond_collapses_without_losing_poison_or_arm_effects() {
    for invert in [false, true] {
        for poison in [false, true] {
            for side_effect in [false, true] {
                let mut t = TestGraph::new();
                t.graph.abi.args.push(PassMode::Direct);
                t.graph.set_value_type(
                    t.graph.start,
                    ValueType::Tuple(vec![
                        ValueType::Control,
                        ValueType::Effect,
                        ValueType::Rust(t.boolean),
                    ]),
                );
                let start = t.graph.start;
                let flag = t.graph.add_node(
                    if poison {
                        Opcode::Undefined
                    } else {
                        Opcode::Projection
                    },
                    NodeId::NONE,
                    if poison {
                        &[]
                    } else {
                        std::slice::from_ref(&start)
                    },
                    if poison { 0 } else { 2 },
                    ValueType::Rust(t.boolean),
                );
                let (mut yes, no) = t.branch(flag);
                let mut yes_effect = t.effect;
                if side_effect {
                    (yes, yes_effect, _) = t.call(yes, "side_effect");
                }
                let yes = t.wrap_control(yes, 3);
                let no = t.wrap_control(no, 2);
                let a = t
                    .graph
                    .add_node(Opcode::Jump, yes, &[], 0, ValueType::Control);
                let b = t
                    .graph
                    .add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
                let region =
                    t.graph
                        .add_node(Opcode::Region, NodeId::NONE, &[a, b], 0, ValueType::Control);
                let effect = t.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[region, yes_effect, t.effect],
                    0,
                    ValueType::Effect,
                );
                let yes_value = t.graph.add_constant(u128::from(!invert), t.boolean);
                let no_value = t.graph.add_constant(u128::from(invert), t.boolean);
                let value = t.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[region, yes_value, no_value],
                    0,
                    ValueType::Rust(t.boolean),
                );
                t.return_value(region, effect, value);
                verify_graph(&mut t.graph).unwrap();
                let before = t.graph.clone();
                optimize_specialized_graph(&mut t.graph).unwrap();
                verify_graph(&mut t.graph).unwrap();
                if !side_effect {
                    assert!(
                        !t.graph.ids().any(|n| t.graph.op(n) == Opcode::If),
                        "{}",
                        t.graph.dump()
                    );
                }
                for input in [0, 1] {
                    let mut outcomes = Vec::new();
                    for g in [&before, &t.graph] {
                        let mut calls = 0;
                        let outcome = Interpreter::new(g).run_values(
                            &[Value::Scalar(input)],
                            &mut |_: &str, _: &[Value]| {
                                calls += 1;
                                Ok(Value::Scalar(17))
                            },
                        );
                        if poison {
                            assert!(matches!(outcome, Err(InterpretError::PoisonObserved(_))));
                        }
                        outcomes.push((format!("{outcome:?}"), calls));
                    }
                    // Poison diagnostics carry different observing node IDs.
                    if !poison {
                        assert_eq!(outcomes[0], outcomes[1]);
                    } else {
                        assert_eq!(outcomes[0].1, outcomes[1].1);
                    }
                }
            }
        }
    }
}

#[test]
fn identity_transmute_preserves_poison_and_removes_representation_roundtrip() {
    for poison in [false, true] {
        let mut t = TestGraph::new();
        let value = if poison {
            t.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(t.integer),
            )
        } else {
            t.graph.add_constant(37, t.integer)
        };
        let transmute = t.graph.add_node(
            Opcode::TransmuteValue,
            t.control,
            &[t.effect, value],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(t.integer),
            ]),
        );
        let control = t.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[transmute],
            0,
            ValueType::Control,
        );
        let effect = t.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[transmute],
            1,
            ValueType::Effect,
        );
        let result = t.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[transmute],
            2,
            ValueType::Rust(t.integer),
        );
        t.return_value(control, effect, result);
        verify_graph(&mut t.graph).unwrap();
        let before = run(&t.graph).0;
        optimize_specialized_graph(&mut t.graph).unwrap();
        verify_graph(&mut t.graph).unwrap();
        let after = run(&t.graph).0;
        if poison {
            assert!(matches!(before, Err(InterpretError::PoisonObserved(_))));
            assert!(matches!(after, Err(InterpretError::PoisonObserved(_))));
        } else {
            assert_eq!(before, after);
        }
        assert!(
            !t.graph
                .ids()
                .any(|n| t.graph.op(n) == Opcode::TransmuteValue)
        );
    }
}

#[test]
fn layout_known_bits_remove_masked_bounds_checks_without_removing_calls() {
    use kfrc_son::{ComparePredicate, Outcome, lower_layout};
    let mut t = TestGraph::new();
    let (control, effect, input) = t.call(t.control, "input");
    t.control = control;
    t.effect = effect;
    let mask = t.graph.add_constant(63, t.integer);
    let index = t.graph.add_node(
        Opcode::BitAnd,
        NodeId::NONE,
        &[input, mask],
        0,
        ValueType::Rust(t.integer),
    );
    let limit = t.graph.add_constant(64, t.integer);
    let condition = t.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[index, limit],
        ComparePredicate::UnsignedLess as u32,
        ValueType::Rust(t.boolean),
    );
    let (yes, no) = t.branch(condition);
    t.return_value(yes, effect, index);
    let (panic_control, panic_effect, panic_value) = t.call(no, "bounds_panic");
    t.return_value(panic_control, panic_effect, panic_value);
    verify_graph(&mut t.graph).unwrap();
    lower_layout(&mut t.graph).unwrap();
    verify_graph(&mut t.graph).unwrap();
    assert!(!t.graph.ids().any(|n| t.graph.op(n) == Opcode::If));
    for input in [0, 63, 64, 255, u64::MAX] {
        let mut calls = 0;
        let outcome = Interpreter::new(&t.graph)
            .run(&[], &mut |name: &str, _: &[u64]| {
                assert_eq!(name, "input");
                calls += 1;
                Ok(input)
            })
            .unwrap();
        assert_eq!(outcome, Outcome::Return(input & 63));
        assert_eq!(calls, 1);
    }
}

#[test]
fn layout_proves_wrapping_slice_length_through_defined_phi_arms() {
    use kfrc_son::{ComparePredicate, Outcome, lower_layout};
    for other_length in [None, Some(8u128)] {
        let mut t = TestGraph::new();
        let (control, effect, input) = t.call(t.control, "input");
        t.control = control;
        t.effect = effect;
        let zero = t.graph.add_constant(0, t.integer);
        let selector = t.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[input, zero],
            ComparePredicate::NotEqual as u32,
            ValueType::Rust(t.boolean),
        );
        let (yes, no) = t.branch(selector);
        let offset = t.graph.add_constant(32, t.integer);
        let end = t.graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[input, offset],
            0,
            ValueType::Rust(t.integer),
        );
        let length = t.graph.add_node(
            Opcode::Sub,
            NodeId::NONE,
            &[end, input],
            0,
            ValueType::Rust(t.integer),
        );
        let other = if let Some(v) = other_length {
            t.graph.add_constant(v, t.integer)
        } else {
            t.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(t.integer),
            )
        };
        let a = t
            .graph
            .add_node(Opcode::Jump, yes, &[], 0, ValueType::Control);
        let b = t
            .graph
            .add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
        t.control = t
            .graph
            .add_node(Opcode::Region, NodeId::NONE, &[a, b], 0, ValueType::Control);
        t.effect = t.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[t.control, effect, effect],
            0,
            ValueType::Effect,
        );
        let length = t.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[t.control, length, other],
            0,
            ValueType::Rust(t.integer),
        );
        let index = t.graph.add_constant(31, t.integer);
        let condition = t.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[index, length],
            ComparePredicate::UnsignedLess as u32,
            ValueType::Rust(t.boolean),
        );
        let (valid, invalid) = t.branch(condition);
        t.return_value(valid, t.effect, offset);
        t.return_value(invalid, t.effect, zero);
        verify_graph(&mut t.graph).unwrap();
        let before = t.graph.clone();
        lower_layout(&mut t.graph).unwrap();
        verify_graph(&mut t.graph).unwrap();
        assert_eq!(
            t.graph
                .ids()
                .filter(|&n| t.graph.op(n) == Opcode::If)
                .count(),
            if other_length.is_none() { 0 } else { 2 }
        );
        for input in [0, 1, u64::MAX - 15, u64::MAX] {
            if input == 0 && other_length.is_none() {
                continue;
            }
            let a = Interpreter::new(&before)
                .run_values(&[], &mut |_: &str, _: &[Value]| Ok(Value::Scalar(input)))
                .unwrap();
            let b = Interpreter::new(&t.graph)
                .run(&[], &mut |_: &str, _: &[u64]| Ok(input))
                .unwrap();
            let expected = if input == 0 { 0 } else { 32 };
            assert_eq!(a, SemanticOutcome::Return(Value::Scalar(expected)));
            assert_eq!(b, Outcome::Return(expected));
        }
    }
}

#[test]
fn redundant_merges_of_one_value_fold_to_that_value() {
    let mut test = TestGraph::new();
    let control = test.control;
    let (after, _effect, value) = test.call(control, "left");
    test.control = after;
    let (threshold_ctl, threshold_effect, threshold) = test.call(after, "right");
    test.control = threshold_ctl;
    let condition = test.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[value, threshold],
        kfrc_son::ComparePredicate::Equal as u32,
        ValueType::Rust(test.boolean),
    );
    let (yes, no) = test.branch(condition);
    // One arm passes through a single-predecessor wrapper with its own Phi.
    let wrapper_jump = test
        .graph
        .add_node(Opcode::Jump, yes, &[], 0, ValueType::Control);
    let wrapper = test.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[wrapper_jump],
        0,
        ValueType::Control,
    );
    let wrapped = test.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[wrapper, value],
        0,
        ValueType::Rust(test.integer),
    );
    let yes_jump = test
        .graph
        .add_node(Opcode::Jump, wrapper, &[], 0, ValueType::Control);
    let no_jump = test
        .graph
        .add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
    let region = test.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[yes_jump, no_jump],
        0,
        ValueType::Control,
    );
    let merged = test.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, wrapped, value],
        0,
        ValueType::Rust(test.integer),
    );
    let merged_effect = test.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, threshold_effect, threshold_effect],
        0,
        ValueType::Effect,
    );
    test.return_value(region, merged_effect, merged);
    verify_graph(&mut test.graph).unwrap();
    let original = test.graph.clone();
    optimize_specialized_graph(&mut test.graph).unwrap();
    verify_graph(&mut test.graph).unwrap();
    assert_eq!(run(&test.graph), run(&original));
    let ret = test
        .graph
        .ids()
        .find(|node| test.graph.op(*node) == Opcode::Return)
        .unwrap();
    let returned = test.graph.resolve(test.graph.input(ret, 1));
    assert_eq!(test.graph.op(returned), Opcode::Projection);
    assert!(
        !test
            .graph
            .ids()
            .any(|node| test.graph.op(node) == Opcode::Phi)
    );
}

/// `assume(false)` guarding a precondition call (as `assume(ub_checks())` does
/// once UB checks are off) makes its path unreachable: the call and the
/// Assume disappear, the live path and its observable call stay, and a
/// non-literal assumption on the live path is retained.
#[test]
fn literal_false_assumption_terminates_its_path() {
    use kfrc_son::ComparePredicate;
    for guarded_call in [false, true] {
        let mut test = TestGraph::new();
        let (probe_ctl, probe_effect, probe) = test.call(test.control, "left");
        let expected = test.graph.add_constant(37, test.integer);
        let matches = test.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[probe, expected],
            ComparePredicate::Equal as u32,
            ValueType::Rust(test.boolean),
        );
        test.control = probe_ctl;
        let (live, dead) = test.branch(matches);
        let assumption = |test: &mut TestGraph, control, effect, predicate| {
            let assume = test.graph.add_node(
                Opcode::Assume,
                control,
                &[effect, predicate],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
            );
            let control = test.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[assume],
                0,
                ValueType::Control,
            );
            let effect = test.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[assume],
                1,
                ValueType::Effect,
            );
            (control, effect)
        };
        // Live path: a data-dependent assumption that must survive.
        let (live, live_effect) = assumption(&mut test, live, probe_effect, matches);
        test.return_value(live, live_effect, probe);
        // Dead path: assume(false), optionally followed by a check call.
        let never = test.graph.add_constant(0, test.boolean);
        let (mut dead, dead_effect) = assumption(&mut test, dead, probe_effect, never);
        if guarded_call {
            test.effect = dead_effect;
            let (after, _, _) = test.call(dead, "precondition_check");
            dead = after;
        }
        test.graph
            .add_node(Opcode::Unreachable, dead, &[], 0, ValueType::Control);
        verify_graph(&mut test.graph).unwrap_or_else(|e| panic!("{e}\n{}", test.graph.dump()));
        let before = run(&test.graph);
        assert_eq!(before.0, Ok(SemanticOutcome::Return(Value::Scalar(37))));

        optimize_specialized_graph(&mut test.graph).unwrap();
        let g = &test.graph;
        let ops: Vec<_> = g.ids().map(|n| g.op(n)).collect();
        let assumptions = g
            .ids()
            .filter(|&n| g.op(n) == Opcode::Assume)
            .map(|n| g.op(g.resolve(g.input(n, 1))))
            .collect::<Vec<_>>();
        assert!(
            !assumptions.contains(&Opcode::Const),
            "literal assumption left behind:\n{}",
            g.dump()
        );
        assert_eq!(
            assumptions.len(),
            1,
            "live assumption must stay:\n{}",
            g.dump()
        );
        assert_eq!(
            ops.iter().filter(|&&op| op == Opcode::Call).count(),
            1,
            "only the live call remains:\n{}",
            g.dump()
        );
        assert_eq!(run(g), before);
    }
}
