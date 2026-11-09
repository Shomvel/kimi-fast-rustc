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
