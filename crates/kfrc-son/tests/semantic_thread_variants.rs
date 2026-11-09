//! Differential tests for semantic predecessor routing, with observable calls.
use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, TyId, TypeShape,
};
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId,
    Opcode, SemanticOutcome, Value, ValueType, optimize_specialized_graph, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    enumeration: TyId,
    control: NodeId,
    effect: NodeId,
    input: NodeId,
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
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "ControlFlow".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            enumeration,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![word], vec![word], vec![word]],
            },
        );
        types.record_semantic_enum(enumeration, vec![11, 29, 37]);
        let mut graph = FunctionGraph::new(
            "thread_variants",
            FunctionAbi {
                args: vec![PassMode::Direct],
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
        let input = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(word),
        );
        Self {
            graph,
            word,
            boolean,
            enumeration,
            control,
            effect,
            input,
        }
    }
    fn eq(&mut self, value: NodeId, bits: u128) -> NodeId {
        let constant = self.graph.add_constant(bits, self.word);
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[value, constant],
            ComparePredicate::Equal as u32,
            ValueType::Rust(self.boolean),
        )
    }
    fn branch(&mut self, control: NodeId, condition: NodeId) -> [NodeId; 2] {
        let branch = self.graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        [0, 1].map(|side| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                side,
                ValueType::Control,
            )
        })
    }
    fn variant(&mut self, variant: u32, payload: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[payload],
            variant,
            ValueType::Rust(self.enumeration),
        )
    }
    fn field(&mut self, value: NodeId, variant: u32) -> NodeId {
        let field = self.graph.intern_variant_field(variant, 0);
        self.graph.add_node(
            Opcode::VariantField,
            NodeId::NONE,
            &[value],
            field,
            ValueType::Rust(self.word),
        )
    }
    fn merge(
        &mut self,
        controls: &[NodeId],
        values: &[NodeId],
        effects: &[NodeId],
    ) -> (NodeId, NodeId, NodeId) {
        let jumps: Vec<_> = controls
            .iter()
            .map(|&c| {
                self.graph
                    .add_node(Opcode::Jump, c, &[], 0, ValueType::Control)
            })
            .collect();
        let region =
            self.graph
                .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
        let mut inputs = vec![region];
        inputs.extend(values);
        let value = self.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &inputs,
            0,
            self.graph.value_type(values[0]).clone(),
        );
        let mut inputs = vec![region];
        inputs.extend(effects);
        let effect = self
            .graph
            .add_node(Opcode::Phi, NodeId::NONE, &inputs, 0, ValueType::Effect);
        (region, value, effect)
    }
    fn call(
        &mut self,
        control: NodeId,
        effect: NodeId,
        value: NodeId,
        name: &str,
    ) -> (NodeId, NodeId, NodeId) {
        let target = self.graph.add_callee(CallTarget {
            symbol: name.into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        let call = self.graph.add_node(
            Opcode::Call,
            control,
            &[effect, value],
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(self.word),
            ]),
        );
        let ctl = self.graph.add_node(
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
            ValueType::Rust(self.word),
        );
        (ctl, effect, value)
    }
    fn ret(&mut self, control: NodeId, effect: NodeId, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            control,
            &[effect, value],
            0,
            ValueType::Control,
        );
    }
    fn count(&self, opcode: Opcode) -> usize {
        self.graph
            .ids()
            .filter(|&n| self.graph.op(n) == opcode)
            .count()
    }
}
fn run(
    graph: &FunctionGraph,
    input: u64,
) -> (Result<SemanticOutcome, InterpretError>, Vec<String>) {
    let mut trace = vec![];
    let result = Interpreter::new(graph).run_values(
        &[Value::Scalar(input)],
        &mut |name: &str, args: &[Value]| {
            trace.push(format!("{name}:{args:?}"));
            Ok(args[0].clone())
        },
    );
    (result, trace)
}

fn diamond() -> (Fixture, NodeId, NodeId, NodeId) {
    let mut f = Fixture::new();
    let condition = f.eq(f.input, 0);
    let [left, right] = f.branch(f.control, condition);
    let (left, left_effect, left_value) = f.call(left, f.effect, f.input, "left");
    let (right, right_effect, right_value) = f.call(right, f.effect, f.input, "right");
    let a = f.variant(0, left_value);
    let b = f.variant(1, right_value);
    let (region, value, effect) = f.merge(&[left, right], &[a, b], &[left_effect, right_effect]);
    (f, region, value, effect)
}

#[test]
fn try_style_dispatch_preserves_calls_payloads_and_reconvergent_phi_edges() {
    let (mut f, region, value, effect) = diamond();
    let tag = f.graph.add_node(
        Opcode::Discriminant,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(f.word),
    );
    let condition = f.eq(tag, 11);
    let [yes, no] = f.branch(region, condition);
    let left = f.field(value, 0);
    let right = f.field(value, 1);
    let (yes, a_effect, a) = f.call(yes, effect, left, "continue");
    let (no, b_effect, b) = f.call(no, effect, right, "break");
    let (end, result, effect) = f.merge(&[yes, no], &[a, b], &[a_effect, b_effect]);
    f.ret(end, effect, result);
    verify_graph(&mut f.graph).unwrap();
    let before: Vec<_> = [0, 1, 17, u64::MAX].map(|x| run(&f.graph, x)).into();
    optimize_specialized_graph(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    for (i, input) in [0, 1, 17, u64::MAX].into_iter().enumerate() {
        assert_eq!(run(&f.graph, input), before[i]);
    }
    assert_eq!(f.count(Opcode::If), 1);
    assert_eq!(f.count(Opcode::Call), 4);
    assert_eq!(f.count(Opcode::VariantField), 0);
}

#[test]
fn pure_values_shared_by_both_leaves_are_specialized_per_phi_edge() {
    let (mut f, region, value, effect) = diamond();
    let tag = f.graph.add_node(
        Opcode::VariantOf,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(f.word),
    );
    let condition = f.eq(tag, 0);
    let [yes, no] = f.branch(region, condition);
    let ten = f.graph.add_constant(10, f.word);
    let shared = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[tag, ten],
        0,
        ValueType::Rust(f.word),
    );
    let (end, result, effect) = f.merge(&[yes, no], &[shared, shared], &[effect, effect]);
    f.ret(end, effect, result);
    verify_graph(&mut f.graph).unwrap();
    let before = [run(&f.graph, 0), run(&f.graph, 3)];
    optimize_specialized_graph(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!([run(&f.graph, 0), run(&f.graph, 3)], before);
    assert_eq!(f.count(Opcode::If), 1);
}

#[test]
fn unknown_incoming_variant_never_infers_a_branch_from_poison() {
    let (mut f, region, value, effect) = diamond();
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.enumeration),
    );
    f.graph.set_input(value, 2, poison);
    let tag = f.graph.add_node(
        Opcode::Discriminant,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(f.word),
    );
    let condition = f.eq(tag, 11);
    let [yes, no] = f.branch(region, condition);
    f.ret(yes, effect, f.input);
    f.ret(no, effect, f.input);
    verify_graph(&mut f.graph).unwrap();
    let before = [run(&f.graph, 0), run(&f.graph, 1)];
    let Err(InterpretError::PoisonObserved(observer)) = &before[1].0 else {
        panic!(
            "expected poison observed by the variant dispatch: {:?}",
            before[1].0
        );
    };
    assert_eq!(f.graph.op(*observer), Opcode::If);
    optimize_specialized_graph(&mut f.graph).unwrap();
    let after = [run(&f.graph, 0), run(&f.graph, 1)];
    assert_eq!(after[0], before[0]);
    let Err(InterpretError::PoisonObserved(observer)) = &after[1].0 else {
        panic!(
            "expected poison observed by the variant dispatch: {:?}",
            after[1].0
        );
    };
    assert_eq!(f.graph.op(*observer), Opcode::If);
    assert_eq!(after[1].1, before[1].1);
    assert_eq!(f.count(Opcode::If), 2);
}

#[test]
fn unused_trap_anchored_at_merge_prevents_bypassing_its_execution() {
    let (mut f, region, value, effect) = diamond();
    let zero = f.graph.add_constant(0, f.word);
    f.graph.add_node(
        Opcode::DivUnsigned,
        region,
        &[f.input, zero],
        0,
        ValueType::Rust(f.word),
    );
    let tag = f.graph.add_node(
        Opcode::Discriminant,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(f.word),
    );
    let condition = f.eq(tag, 11);
    let [yes, no] = f.branch(region, condition);
    f.ret(yes, effect, f.input);
    f.ret(no, effect, f.input);
    verify_graph(&mut f.graph).unwrap();
    let before = run(&f.graph, 0);
    let division_reason = |outcome: &Result<SemanticOutcome, InterpretError>| {
        let Err(InterpretError::UndefinedBehaviour(message)) = outcome else {
            panic!("expected division trap: {outcome:?}");
        };
        let (node, reason) = message
            .split_once(": ")
            .expect("node-qualified division error");
        assert!(
            node.strip_prefix('n')
                .is_some_and(|id| id.parse::<u32>().is_ok())
        );
        assert_eq!(reason, "division by zero or overflow");
    };
    division_reason(&before.0);
    optimize_specialized_graph(&mut f.graph).unwrap();
    let after = run(&f.graph, 0);
    division_reason(&after.0);
    assert_eq!(after.1, before.1);
    assert_eq!(f.count(Opcode::If), 2);
    assert_eq!(f.count(Opcode::DivUnsigned), 1);
}

#[test]
fn grouped_predecessors_keep_distinct_payloads_and_effects() {
    let mut f = Fixture::new();
    let is_zero = f.eq(f.input, 0);
    let [zero, rest] = f.branch(f.control, is_zero);
    let is_one = f.eq(f.input, 1);
    let [one, other] = f.branch(rest, is_one);
    let mut controls = vec![];
    let mut values = vec![];
    let mut effects = vec![];
    for (index, (control, name)) in [(zero, "zero"), (one, "one"), (other, "other")]
        .into_iter()
        .enumerate()
    {
        let (control, effect, value) = f.call(control, f.effect, f.input, name);
        controls.push(control);
        effects.push(effect);
        values.push(f.variant(if index == 2 { 1 } else { 0 }, value));
    }
    let (region, value, effect) = f.merge(&controls, &values, &effects);
    let tag = f.graph.add_node(
        Opcode::Discriminant,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(f.word),
    );
    let is_continue = f.eq(tag, 11);
    let [yes, no] = f.branch(region, is_continue);
    for (control, variant, name) in [(yes, 0, "continue"), (no, 1, "break")] {
        let field = f.field(value, variant);
        let (control, effect, result) = f.call(control, effect, field, name);
        f.ret(control, effect, result);
    }
    verify_graph(&mut f.graph).unwrap();
    let before = [
        run(&f.graph, 0),
        run(&f.graph, 1),
        run(&f.graph, 2),
        run(&f.graph, 999),
    ];
    optimize_specialized_graph(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(
        [
            run(&f.graph, 0),
            run(&f.graph, 1),
            run(&f.graph, 2),
            run(&f.graph, 999)
        ],
        before
    );
    assert_eq!(f.count(Opcode::If), 2);
    assert_eq!(f.count(Opcode::Call), 5);
}

#[test]
fn chained_discriminant_tests_thread_new_regions_without_copying_calls() {
    let mut f = Fixture::new();
    let is_zero = f.eq(f.input, 0);
    let [zero, rest] = f.branch(f.control, is_zero);
    let is_one = f.eq(f.input, 1);
    let [one, other] = f.branch(rest, is_one);
    let values: Vec<_> = (0..3).map(|variant| f.variant(variant, f.input)).collect();
    let (region, value, effect) = f.merge(&[zero, one, other], &values, &[f.effect; 3]);
    let tag = f.graph.add_node(
        Opcode::Discriminant,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(f.word),
    );
    let is_first = f.eq(tag, 11);
    let [first, rest] = f.branch(region, is_first);
    let is_second = f.eq(tag, 29);
    let [second, third] = f.branch(rest, is_second);
    for (control, variant, name) in [
        (first, 0, "first"),
        (second, 1, "second"),
        (third, 2, "third"),
    ] {
        let field = f.field(value, variant);
        let (control, effect, result) = f.call(control, effect, field, name);
        f.ret(control, effect, result);
    }
    verify_graph(&mut f.graph).unwrap();
    let before = [run(&f.graph, 0), run(&f.graph, 1), run(&f.graph, 2)];
    optimize_specialized_graph(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(
        [run(&f.graph, 0), run(&f.graph, 1), run(&f.graph, 2)],
        before
    );
    assert_eq!(f.count(Opcode::If), 2);
    assert_eq!(f.count(Opcode::Call), 3);
    assert_eq!(f.count(Opcode::VariantField), 0);
}

#[test]
fn logical_variant_switch_threads_predecessors_and_retires_exact_variant_witnesses() {
    let (mut f, region, value, effect) = diamond();
    let cases = f.graph.variant_switches.len() as u32;
    f.graph.variant_switches.push(vec![0, 1]);
    let switch = f.graph.add_node(
        Opcode::VariantSwitch,
        region,
        &[value],
        cases,
        ValueType::Tuple(vec![ValueType::Control; 3]),
    );
    for variant in 0..2 {
        let control = f.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[switch],
            variant,
            ValueType::Control,
        );
        let witness = f.graph.add_node(
            Opcode::VariantWitness,
            control,
            &[value, control],
            variant,
            ValueType::Witness,
        );
        let field = f.field(value, variant);
        f.graph.set_inputs(field, &[value, witness]);
        let (control, effect, result) = f.call(
            control,
            effect,
            field,
            if variant == 0 { "continue" } else { "break" },
        );
        f.ret(control, effect, result);
    }
    let otherwise = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[switch],
        2,
        ValueType::Control,
    );
    f.graph
        .add_node(Opcode::Unreachable, otherwise, &[], 0, ValueType::Control);
    verify_graph(&mut f.graph).unwrap();
    let before = [run(&f.graph, 0), run(&f.graph, 13)];
    optimize_specialized_graph(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!([run(&f.graph, 0), run(&f.graph, 13)], before);
    assert_eq!(f.count(Opcode::VariantSwitch), 0);
    assert_eq!(f.count(Opcode::VariantWitness), 0);
    assert_eq!(f.count(Opcode::If), 1);
    assert_eq!(f.count(Opcode::Call), 4);
}

#[test]
fn constructor_switch_selects_explicit_and_default_edges_without_layout() {
    for variant in 0..3 {
        let mut f = Fixture::new();
        let value = f.variant(variant, f.input);
        let cases = f.graph.variant_switches.len() as u32;
        f.graph.variant_switches.push(vec![2, 0]);
        let switch = f.graph.add_node(
            Opcode::VariantSwitch,
            f.control,
            &[value],
            cases,
            ValueType::Tuple(vec![ValueType::Control; 3]),
        );
        for (edge, selected) in [2, 0, 1].into_iter().enumerate() {
            let control = f.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[switch],
                edge as u32,
                ValueType::Control,
            );
            let witness = f.graph.add_node(
                Opcode::VariantWitness,
                control,
                &[value, control],
                selected,
                ValueType::Witness,
            );
            let payload = f.field(value, selected);
            f.graph.set_inputs(payload, &[value, witness]);
            let (control, effect, result) = f.call(
                control,
                f.effect,
                payload,
                ["zero", "one", "two"][selected as usize],
            );
            f.ret(control, effect, result);
        }
        verify_graph(&mut f.graph).unwrap();
        let before = [run(&f.graph, 0), run(&f.graph, u64::MAX)];
        optimize_specialized_graph(&mut f.graph).unwrap();
        assert_eq!([run(&f.graph, 0), run(&f.graph, u64::MAX)], before);
        assert_eq!(f.count(Opcode::VariantSwitch), 0);
        assert_eq!(f.count(Opcode::Call), 1);
    }
}
