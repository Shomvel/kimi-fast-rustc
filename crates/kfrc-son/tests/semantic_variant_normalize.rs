//! R preparation exposes semantic enum control after promotion/GVN.
use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, TyId, TypeShape,
};
use kfrc_son::specialize::RustDefinition;
use kfrc_son::{
    ComparePredicate, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode,
    RustSignature, SemanticOutcome, Value, ValueType, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    arg: NodeId,
    yes: NodeId,
    no: NodeId,
    effect: NodeId,
}
impl Fixture {
    fn new(predicate: ComparePredicate, reversed: bool, constant: u128) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            None,
        );
        let tag = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: true,
            },
            None,
        );
        let boolean = types.intern(TypeShape::Bool, None);
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "Choice".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            enumeration,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![word], vec![word]],
            },
        );
        types.record_semantic_enum(enumeration, vec![u128::MAX, 7]);
        let mut graph = FunctionGraph::new(
            "normalize_variant",
            FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Rust;
        graph.rust_signature = Some(RustSignature {
            args: vec![enumeration],
            ret: word,
        });
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(enumeration),
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
        let arg = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(enumeration),
        );
        let discriminant = graph.add_node(
            Opcode::Discriminant,
            NodeId::NONE,
            &[arg],
            0,
            ValueType::Rust(tag),
        );
        let constant = graph.add_constant(constant, tag);
        let inputs = if reversed {
            [constant, discriminant]
        } else {
            [discriminant, constant]
        };
        let comparison = graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &inputs,
            predicate as u32,
            ValueType::Rust(boolean),
        );
        let branch = graph.add_node(
            Opcode::If,
            control,
            &[comparison],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let yes = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            0,
            ValueType::Control,
        );
        let no = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            1,
            ValueType::Control,
        );
        Self {
            graph,
            word,
            arg,
            yes,
            no,
            effect,
        }
    }
    fn field(&mut self, variant: u32) -> NodeId {
        let payload = self.graph.intern_variant_field(variant, 0);
        self.graph.add_node(
            Opcode::VariantField,
            NodeId::NONE,
            &[self.arg],
            payload,
            ValueType::Rust(self.word),
        )
    }
    fn ret(&mut self, control: NodeId, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
    }
}
fn run(
    graph: &FunctionGraph,
    variant: u32,
    payload: u64,
) -> Result<SemanticOutcome, InterpretError> {
    let mut graph = graph.clone();
    // This concrete, call-free fixture needs no type or ABI substitution.
    // Drop the R-only signature when evaluating its values at S.
    graph.level = GraphLevel::Specialized;
    graph.rust_signature = None;
    verify_graph(&mut graph).unwrap();
    Interpreter::new(&graph).run_values(
        &[Value::Variant {
            variant,
            fields: vec![Value::Scalar(payload)],
        }],
        &mut |name: &str, _: &[Value]| Err(InterpretError::UnknownCallee(name.into())),
    )
}

#[test]
fn equal_and_not_equal_preserve_edge_meaning_and_witness_only_matching_variant() {
    for predicate in [ComparePredicate::Equal, ComparePredicate::NotEqual] {
        for reversed in [false, true] {
            let mut f = Fixture::new(predicate, reversed, 255);
            let left = f.field(0);
            let right = f.field(1);
            if predicate == ComparePredicate::Equal {
                f.ret(f.yes, left);
                f.ret(f.no, right);
            } else {
                f.ret(f.yes, right);
                f.ret(f.no, left);
            }
            let original = f.graph.clone();
            let definition = RustDefinition::prepare(f.graph).unwrap();
            let graph = definition.graph();
            assert_eq!(
                graph
                    .ids()
                    .filter(|&n| graph.op(n) == Opcode::VariantSwitch)
                    .count(),
                1
            );
            assert!(!graph.ids().any(|n| graph.op(n) == Opcode::If));
            assert_eq!(
                graph
                    .ids()
                    .filter(|&n| graph.op(n) == Opcode::VariantWitness)
                    .count(),
                2
            );
            for field in graph.ids().filter(|&n| graph.op(n) == Opcode::VariantField) {
                let witness = graph.input(field, 1);
                assert_eq!(graph.op(witness), Opcode::VariantWitness);
                assert_eq!(graph.value_type(witness), &ValueType::Witness);
                assert_eq!(graph.input(witness, 0), graph.input(field, 0));
                assert_eq!(
                    graph.payload(witness),
                    graph.variant_fields[graph.payload(field) as usize].0
                );
            }
            for variant in 0..2 {
                assert_eq!(run(graph, variant, 81), run(&original, variant, 81));
            }
        }
    }
}

#[test]
fn shared_inactive_field_after_reconvergence_is_not_given_a_branch_fact() {
    let mut f = Fixture::new(ComparePredicate::Equal, false, 255);
    let jumps: Vec<_> = [f.yes, f.no]
        .map(|control| {
            f.graph
                .add_node(Opcode::Jump, control, &[], 0, ValueType::Control)
        })
        .into();
    let merge = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
    let field = f.field(0);
    f.ret(merge, field);
    let definition = RustDefinition::prepare(f.graph).unwrap();
    let graph = definition.graph();
    assert!(!graph.ids().any(|n| graph.op(n) == Opcode::VariantWitness));
    assert!(matches!(
        run(graph, 1, 91),
        Err(InterpretError::PoisonObserved(_))
    ));
    assert_eq!(
        run(graph, 0, 91),
        Ok(SemanticOutcome::Return(Value::Scalar(91)))
    );
}

#[test]
fn phi_incoming_edges_supply_precise_use_execution_points() {
    let mut f = Fixture::new(ComparePredicate::Equal, false, 255);
    let left = f.field(0);
    let right = f.field(1);
    let yes = f
        .graph
        .add_node(Opcode::Jump, f.yes, &[], 0, ValueType::Control);
    let no = f
        .graph
        .add_node(Opcode::Jump, f.no, &[], 0, ValueType::Control);
    let merge = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[yes, no],
        0,
        ValueType::Control,
    );
    let value = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[merge, left, right],
        0,
        ValueType::Rust(f.word),
    );
    f.ret(merge, value);
    let definition = RustDefinition::prepare(f.graph).unwrap();
    let graph = definition.graph();
    assert_eq!(
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::VariantWitness)
            .count(),
        2
    );
    for variant in 0..2 {
        assert_eq!(
            run(graph, variant, 13),
            Ok(SemanticOutcome::Return(Value::Scalar(13)))
        );
    }
}

#[test]
fn invalid_source_discriminant_and_disabled_optimization_keep_original_control() {
    for (constant, optimize) in [(3, true), (255, false)] {
        let mut f = Fixture::new(ComparePredicate::Equal, false, constant);
        let zero = f.graph.add_constant(0, f.word);
        let one = f.graph.add_constant(1, f.word);
        f.ret(f.yes, zero);
        f.ret(f.no, one);
        let definition = RustDefinition::prepare_with_optimization(f.graph, optimize).unwrap();
        assert!(
            definition
                .graph()
                .ids()
                .any(|n| definition.graph().op(n) == Opcode::If)
        );
        assert!(
            !definition
                .graph()
                .ids()
                .any(|n| definition.graph().op(n) == Opcode::VariantSwitch)
        );
    }
}
