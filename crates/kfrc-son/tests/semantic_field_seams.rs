use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, TyId, TypeShape};
use kfrc_son::{
    ComparePredicate, FunctionGraph, Interpreter, NodeId, Opcode, SemanticOutcome, Value,
    ValueType, optimize_specialized_seams, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    index: NodeId,
    pair: NodeId,
    control: NodeId,
    effect: NodeId,
}

impl Fixture {
    fn new() -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            None,
        );
        let boolean = types.intern(TypeShape::Bool, None);
        let pair = types.intern(TypeShape::Tuple(vec![word, word]), None);
        let mut graph = FunctionGraph::new(
            "field_seam",
            FunctionAbi {
                args: vec![PassMode::Direct, PassMode::Pair],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
                ValueType::Rust(pair),
            ]),
        );
        graph.start = start;
        let mut project =
            |slot, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[start], slot, ty);
        let control = project(0, ValueType::Control);
        let effect = project(1, ValueType::Effect);
        let index = project(2, ValueType::Rust(word));
        let pair = project(3, ValueType::Rust(pair));
        Self {
            graph,
            word,
            boolean,
            index,
            pair,
            control,
            effect,
        }
    }
    fn field(&mut self, index: u32) -> NodeId {
        self.graph.add_node(
            Opcode::Field,
            NodeId::NONE,
            &[self.pair],
            index,
            ValueType::Rust(self.word),
        )
    }
    fn less(&mut self, a: NodeId, b: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[a, b],
            ComparePredicate::UnsignedLess as u32,
            ValueType::Rust(self.boolean),
        )
    }
    fn branch(&mut self, at: NodeId, condition: NodeId) -> [NodeId; 2] {
        let branch = self.graph.add_node(
            Opcode::If,
            at,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        [0, 1].map(|slot| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                slot,
                ValueType::Control,
            )
        })
    }
    fn ret(&mut self, at: NodeId, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            at,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
    }
}

fn run(graph: &FunctionGraph, index: u64, len: u64, value: u64) -> SemanticOutcome {
    Interpreter::new(graph)
        .run_values(
            &[
                Value::Scalar(index),
                Value::Aggregate(vec![Value::Scalar(len), Value::Scalar(value)]),
            ],
            &mut |symbol: &str, _: &[Value]| {
                Err(kfrc_son::InterpretError::UnknownCallee(symbol.into()))
            },
        )
        .unwrap()
}

#[test]
fn seeded_field_identity_enables_scoped_comparison_without_rescanning_arithmetic() {
    let mut f = Fixture::new();
    let original = f.field(0);
    let condition = f.less(f.index, original);
    let [body, exit] = f.branch(f.control, condition);
    let late = f.field(0);
    let repeated = f.less(f.index, late);
    let [yes, no] = f.branch(body, repeated);
    let two = f.graph.add_constant(2, f.word);
    let mut unrelated = f.index;
    for _ in 0..120 {
        unrelated = f.graph.add_node(
            Opcode::Mul,
            NodeId::NONE,
            &[unrelated, two],
            0,
            ValueType::Rust(f.word),
        );
    }
    f.ret(exit, unrelated);
    f.ret(yes, unrelated);
    let bad = f.graph.add_constant(99, f.word);
    f.ret(no, bad);
    verify_graph(&mut f.graph).unwrap();
    let inputs = [
        (0, 0, 7),
        (0, 1, 7),
        (1, 1, 7),
        (2, 9, 7),
        (u64::MAX, u64::MAX, 7),
    ];
    let before: Vec<_> = inputs
        .iter()
        .map(|&(i, n, v)| run(&f.graph, i, n, v))
        .collect();
    let stats = optimize_specialized_seams(&mut f.graph, &[late], &[]).unwrap();
    assert_eq!(stats.fields_folded, 1, "{stats:?}");
    // Distinct Compare nodes need RS3.3 implication after their operands merge.
    assert_eq!(stats.conditions_folded, 1, "{stats:?}");
    assert!(stats.rule_nodes_visited < 50, "{stats:?}");
    assert!(stats.rule_nodes_indexed >= 120, "{stats:?}");
    verify_graph(&mut f.graph).unwrap();
    for ((i, n, v), expected) in inputs.into_iter().zip(before) {
        assert_eq!(run(&f.graph, i, n, v), expected);
    }
}

#[test]
fn unseeded_duplicate_fields_are_left_to_their_owner() {
    let mut f = Fixture::new();
    let first = f.field(0);
    let second = f.field(0);
    let sum = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[first, second],
        0,
        ValueType::Rust(f.word),
    );
    f.ret(f.control, sum);
    verify_graph(&mut f.graph).unwrap();
    let stats = optimize_specialized_seams(&mut f.graph, &[], &[]).unwrap();
    assert_eq!(stats.fields_folded, 0, "{stats:?}");
    assert_eq!(
        f.graph
            .ids()
            .filter(|&n| f.graph.op(n) == Opcode::Field)
            .count(),
        2
    );
    assert_eq!(
        run(&f.graph, 0, 17, 23),
        SemanticOutcome::Return(Value::Scalar(34))
    );
}

#[test]
fn different_fields_are_not_merged() {
    let mut f = Fixture::new();
    let first = f.field(0);
    let second = f.field(1);
    let sum = f.graph.add_node(
        Opcode::Sub,
        NodeId::NONE,
        &[first, second],
        0,
        ValueType::Rust(f.word),
    );
    f.ret(f.control, sum);
    verify_graph(&mut f.graph).unwrap();
    let stats = optimize_specialized_seams(&mut f.graph, &[second], &[]).unwrap();
    assert_eq!(stats.fields_folded, 0, "{stats:?}");
    assert_eq!(
        run(&f.graph, 0, 23, 17),
        SemanticOutcome::Return(Value::Scalar(6))
    );
}

#[test]
fn demanded_end_field_is_invariant_while_the_loop_index_changes() {
    let mut f = Fixture::new();
    let end = f.field(0);
    let pair_ty = f.graph.rust_type(f.pair).unwrap();
    let initial = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[f.index, end],
        0,
        ValueType::Rust(pair_ty),
    );
    let entry = f
        .graph
        .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let state = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, initial],
        0,
        ValueType::Rust(pair_ty),
    );
    let index = f.graph.add_node(
        Opcode::Field,
        NodeId::NONE,
        &[state],
        0,
        ValueType::Rust(f.word),
    );
    let limit = f.graph.add_node(
        Opcode::Field,
        NodeId::NONE,
        &[state],
        1,
        ValueType::Rust(f.word),
    );
    let condition = f.less(index, limit);
    let [body, exit] = f.branch(header, condition);
    let one = f.graph.add_constant(1, f.word);
    let next = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[index, one],
        0,
        ValueType::Rust(f.word),
    );
    let updated = f.graph.add_node(
        Opcode::InsertField,
        NodeId::NONE,
        &[state, next],
        0,
        ValueType::Rust(pair_ty),
    );
    let back = f
        .graph
        .add_node(Opcode::Jump, body, &[], 0, ValueType::Control);
    f.graph.set_inputs(header, &[entry, back]);
    f.graph.set_inputs(state, &[header, initial, updated]);
    f.ret(exit, index);
    verify_graph(&mut f.graph).unwrap();
    let inputs = [(0, 0), (0, 5), (2, 5), (9, 5)];
    let before: Vec<_> = inputs
        .iter()
        .map(|&(i, n)| run(&f.graph, i, n, 99))
        .collect();
    let stats = optimize_specialized_seams(&mut f.graph, &[limit, index], &[]).unwrap();
    assert_eq!(stats.fields_folded, 1, "{stats:?}");
    assert!(f.graph.ids().any(|node| f.graph.op(node) == Opcode::Field
        && f.graph.op(f.graph.input(node, 0)) == Opcode::Phi
        && f.graph.payload(node) == 0));
    assert!(!f.graph.ids().any(|node| f.graph.op(node) == Opcode::Field
        && f.graph.op(f.graph.input(node, 0)) == Opcode::Phi
        && f.graph.payload(node) == 1));
    for ((i, n), expected) in inputs.into_iter().zip(before) {
        assert_eq!(run(&f.graph, i, n, 99), expected);
    }
}
