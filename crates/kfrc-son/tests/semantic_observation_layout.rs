//! Observations must not rebuild inactive enum trees during layout lowering.
#[path = "support/refinement.rs"]
mod refinement;

use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, SizeAlign, TyId,
    TypeShape,
};
use kfrc_son::{
    FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode, Outcome,
    SemanticOutcome, Value, ValueType, lower_layout, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    enumeration: TyId,
    arg: NodeId,
    control: NodeId,
    effect: NodeId,
}
impl Fixture {
    fn new(variants: usize) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "ManyPayloadVariants".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            enumeration,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![word, word]; variants],
            },
        );
        types.record_semantic_enum(enumeration, (0..variants as u128).collect());
        let mut graph = FunctionGraph::new(
            "observation_layout",
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
        let arg = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(word),
        );
        Self {
            graph,
            word,
            enumeration,
            arg,
            control,
            effect,
        }
    }
    fn variant(&mut self, variant: u32, fields: &[NodeId]) -> NodeId {
        self.graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            fields,
            variant,
            ValueType::Rust(self.enumeration),
        )
    }
    fn observe(&mut self, value: NodeId) {
        let observer = self.graph.add_node(
            Opcode::RequireDefined,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[observer],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[observer],
            1,
            ValueType::Effect,
        );
    }
    fn finish(&mut self) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, self.arg],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
    }
    fn lower(&mut self) {
        lower_layout(&mut self.graph).unwrap();
        verify_graph(&mut self.graph).unwrap();
    }
    fn count(&self, op: Opcode) -> usize {
        self.graph.ids().filter(|&n| self.graph.op(n) == op).count()
    }
}
fn semantic(graph: &FunctionGraph, value: u64) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(&[Value::Scalar(value)], &mut |name: &str, _: &[Value]| {
        Err(InterpretError::UnknownCallee(name.into()))
    })
}
fn physical(graph: &FunctionGraph, value: u64) -> Result<Outcome, InterpretError> {
    Interpreter::new(graph).run(&[value], &mut |name: &str, _: &[u64]| {
        Err(InterpretError::UnknownCallee(name.into()))
    })
}

#[test]
fn known_variant_observation_cost_is_independent_of_inactive_variant_count() {
    let mut size = None;
    for variants in [2, 16, 128] {
        let mut f = Fixture::new(variants);
        let value = f.variant(variants as u32 - 1, &[f.arg, f.arg]);
        f.observe(value);
        f.finish();
        assert_eq!(
            semantic(&f.graph, 37),
            Ok(SemanticOutcome::Return(Value::Scalar(37)))
        );
        f.lower();
        assert_eq!(physical(&f.graph, 37), Ok(Outcome::Return(37)));
        assert_eq!(f.count(Opcode::If), 0);
        assert_eq!(f.count(Opcode::RequireDefined), 0);
        assert_eq!(f.graph.len(), *size.get_or_insert(f.graph.len()));
    }
}

#[test]
fn active_poison_is_observed_while_inactive_payloads_never_build_branches() {
    let mut f = Fixture::new(16);
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    let value = f.variant(7, &[f.arg, poison]);
    f.observe(value);
    f.finish();
    assert!(matches!(
        semantic(&f.graph, 19),
        Err(InterpretError::PoisonObserved(_))
    ));
    f.lower();
    // Source UB diagnostics remain in S, not in published representation work.
    assert_eq!(physical(&f.graph, 19), Ok(Outcome::Return(19)));
    assert_eq!(f.count(Opcode::If), 0);
    assert_eq!(f.count(Opcode::RequireDefined), 0);
}

#[test]
fn nested_known_variants_observe_only_the_active_leaves() {
    let mut f = Fixture::new(16);
    let outer = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "NestedError".into(),
        },
        None,
    );
    f.graph.oracle.record_semantic_aggregate(
        outer,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![f.enumeration]; 16],
        },
    );
    f.graph
        .oracle
        .record_semantic_enum(outer, (0..16).collect());
    let inner = f.variant(11, &[f.arg, f.arg]);
    let value = f.graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[inner],
        9,
        ValueType::Rust(outer),
    );
    f.observe(value);
    f.finish();
    assert_eq!(
        semantic(&f.graph, 51),
        Ok(SemanticOutcome::Return(Value::Scalar(51)))
    );
    f.lower();
    assert_eq!(physical(&f.graph, 51), Ok(Outcome::Return(51)));
    assert_eq!(f.count(Opcode::If), 0);
    assert_eq!(f.count(Opcode::RequireDefined), 0);
}

fn merged_variants(f: &mut Fixture, alternatives: [NodeId; 2]) -> NodeId {
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let zero = f.graph.add_constant(0, f.word);
    let condition = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[f.arg, zero],
        kfrc_son::ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let jumps: Vec<_> = (0..2)
        .map(|side| {
            let control = f.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                side,
                ValueType::Control,
            );
            f.graph
                .add_node(Opcode::Jump, control, &[], 0, ValueType::Control)
        })
        .collect();
    f.control = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
    f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[f.control, alternatives[0], alternatives[1]],
        0,
        ValueType::Rust(f.enumeration),
    )
}

#[test]
fn equal_variant_tags_across_phi_need_no_additional_observation_branches() {
    let mut f = Fixture::new(16);
    let left = f.variant(9, &[f.arg, f.arg]);
    let right = f.variant(9, &[f.arg, f.arg]);
    let value = merged_variants(&mut f, [left, right]);
    f.observe(value);
    f.finish();
    f.lower();
    assert_eq!(f.count(Opcode::If), 1);
    for value in [0, 1, 99] {
        assert_eq!(physical(&f.graph, value), Ok(Outcome::Return(value)));
    }
}

#[test]
fn conditional_payload_observation_does_not_leak_definedness_to_other_variants() {
    let mut f = Fixture::new(2);
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    let left = f.variant(0, &[f.arg, f.arg]);
    let right = f.variant(1, &[f.arg, poison]);
    let value = merged_variants(&mut f, [left, right]);
    f.observe(value);
    f.finish();
    assert_eq!(
        semantic(&f.graph, 0),
        Ok(SemanticOutcome::Return(Value::Scalar(0)))
    );
    assert!(matches!(
        semantic(&f.graph, 1),
        Err(InterpretError::PoisonObserved(_))
    ));
    f.lower();
    assert_eq!(physical(&f.graph, 0), Ok(Outcome::Return(0)));
    assert_eq!(physical(&f.graph, 1), Ok(Outcome::Return(1)));
}

#[test]
fn deep_known_variant_construction_never_materializes_inactive_payload_trees() {
    let mut f = Fixture::new(8);
    let mut value = f.variant(7, &[f.arg, f.arg]);
    let mut ty = f.enumeration;
    // Expanding all inactive payloads would require 8^10 nested trees even
    // though this graph owns a single active scalar along its constructor path.
    for depth in 0..10 {
        let outer = f.graph.oracle.intern(
            TypeShape::Adt {
                name: format!("SparseLevel{depth}"),
            },
            None,
        );
        f.graph.oracle.record_semantic_aggregate(
            outer,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![ty]; 8],
            },
        );
        f.graph.oracle.record_semantic_enum(outer, (0..8).collect());
        value = f.graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[value],
            depth % 8,
            ValueType::Rust(outer),
        );
        ty = outer;
    }
    f.observe(value);
    f.finish();
    assert_eq!(
        semantic(&f.graph, 73),
        Ok(SemanticOutcome::Return(Value::Scalar(73)))
    );
    f.lower();
    assert_eq!(physical(&f.graph, 73), Ok(Outcome::Return(73)));
    assert_eq!(f.count(Opcode::If), 0);
    assert_eq!(f.count(Opcode::Undefined), 0);
    assert_eq!(f.count(Opcode::RequireDefined), 0);
    assert!(f.graph.len() < 20);
}

#[test]
fn inactive_known_variant_extraction_stays_poison_until_observed() {
    for observe in [false, true] {
        let mut f = Fixture::new(16);
        let value = f.variant(7, &[f.arg, f.arg]);
        let field = f.graph.intern_variant_field(3, 0);
        let inactive = f.graph.add_node(
            Opcode::VariantField,
            NodeId::NONE,
            &[value],
            field,
            ValueType::Rust(f.word),
        );
        if observe {
            f.observe(inactive);
        }
        f.finish();
        let before = semantic(&f.graph, 99);
        f.lower();
        let after = physical(&f.graph, 99);
        if observe {
            assert!(matches!(before, Err(InterpretError::PoisonObserved(_))));
            assert_eq!(after, Ok(Outcome::Return(99)));
        } else {
            assert_eq!(before, Ok(SemanticOutcome::Return(Value::Scalar(99))));
            assert_eq!(after, Ok(Outcome::Return(99)));
        }
        assert_eq!(f.count(Opcode::If), 0);
    }
}

fn witness(f: &mut Fixture, value: NodeId, variant: u32) -> NodeId {
    f.graph.add_node(
        Opcode::VariantWitness,
        f.control,
        &[value, f.control],
        variant,
        ValueType::Witness,
    )
}

fn assume_variant(f: &mut Fixture, value: NodeId, variant: u32) -> NodeId {
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let index_ty = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let index = f.graph.add_node(
        Opcode::VariantOf,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(index_ty),
    );
    let expected = f.graph.add_constant(u128::from(variant), index_ty);
    let condition = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[index, expected],
        kfrc_son::ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let assume = f.graph.add_node(
        Opcode::Assume,
        f.control,
        &[f.effect, condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    f.control = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[assume],
        0,
        ValueType::Control,
    );
    f.effect = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[assume],
        1,
        ValueType::Effect,
    );
    witness(f, value, variant)
}

fn witnessed_field(
    f: &mut Fixture,
    value: NodeId,
    witness: NodeId,
    variant: u32,
    ty: TyId,
) -> NodeId {
    let field = f.graph.intern_variant_field(variant, 0);
    f.graph.add_node(
        Opcode::VariantField,
        NodeId::NONE,
        &[value, witness],
        field,
        ValueType::Rust(ty),
    )
}

#[test]
fn witnessed_dynamic_field_reuses_payload_without_leaf_pins() {
    let mut f = Fixture::new(2);
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    let left = f.variant(0, &[poison, poison]);
    let right = f.variant(1, &[f.arg, f.arg]);
    let value = merged_variants(&mut f, [left, right]);
    let cases = f.graph.variant_switches.len() as u32;
    f.graph.variant_switches.push(vec![1]);
    let branch = f.graph.add_node(
        Opcode::VariantSwitch,
        f.control,
        &[value],
        cases,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let yes = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        0,
        ValueType::Control,
    );
    let no = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        1,
        ValueType::Control,
    );
    f.graph.add_node(
        Opcode::Return,
        no,
        &[f.effect, f.arg],
        0,
        ValueType::Control,
    );
    f.control = yes;
    let proof = witness(&mut f, value, 1);
    let word = f.word;
    let field = witnessed_field(&mut f, value, proof, 1, word);
    f.graph.add_node(
        Opcode::Return,
        f.control,
        &[f.effect, field],
        0,
        ValueType::Control,
    );
    verify_graph(&mut f.graph).unwrap();
    for arg in [0, 1, 37] {
        assert_eq!(
            semantic(&f.graph, arg),
            Ok(SemanticOutcome::Return(Value::Scalar(arg)))
        );
    }
    f.lower();
    assert_eq!(f.count(Opcode::If), 2);
    assert_eq!(f.count(Opcode::VariantWitness), 0);
    assert_eq!(f.count(Opcode::Load), 0);
    assert_eq!(f.count(Opcode::Store), 0);
    for arg in [0, 1, 37] {
        assert_eq!(physical(&f.graph, arg), Ok(Outcome::Return(arg)));
    }
}

#[test]
fn assumed_variant_is_source_domain_and_preserves_defined_extraction() {
    for active_poison in [false, true] {
        let mut f = Fixture::new(2);
        let payload = if active_poison {
            f.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.word),
            )
        } else {
            f.arg
        };
        let left = f.variant(0, &[f.arg, f.arg]);
        let right = f.variant(1, &[payload, payload]);
        let value = merged_variants(&mut f, [left, right]);
        let proof = assume_variant(&mut f, value, 1);
        let word = f.word;
        let field = witnessed_field(&mut f, value, proof, 1, word);
        f.graph.add_node(
            Opcode::Return,
            f.control,
            &[f.effect, field],
            0,
            ValueType::Control,
        );
        verify_graph(&mut f.graph).unwrap();
        let false_source = semantic(&f.graph, 0);
        let true_source = semantic(&f.graph, 7);
        assert!(matches!(
            false_source,
            Err(InterpretError::UndefinedBehaviour(_))
        ));
        if active_poison {
            assert!(matches!(
                true_source,
                Err(InterpretError::PoisonObserved(_))
            ));
        } else {
            assert_eq!(true_source, Ok(SemanticOutcome::Return(Value::Scalar(7))));
        }
        f.lower();
        assert_eq!(f.count(Opcode::If), 1);
        assert_eq!(f.count(Opcode::Assume), 0);
        assert_eq!(f.count(Opcode::VariantWitness), 0);
        for (source, arg) in [(false_source, 0), (true_source, 7)] {
            refinement::assert_release_refines(
                &source,
                &physical(&f.graph, arg),
                |source, target| matches!((source, target), (SemanticOutcome::Return(Value::Scalar(a)), Outcome::Return(b)) if a == b),
            );
        }
    }
}

#[test]
fn unused_assumption_checks_source_poison_but_never_generates_representation() {
    for poison in [false, true] {
        let mut f = Fixture::new(2);
        let value = if poison {
            f.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.enumeration),
            )
        } else {
            f.variant(0, &[f.arg, f.arg])
        };
        assume_variant(&mut f, value, 1);
        f.finish();
        let source = semantic(&f.graph, 9);
        assert!(matches!(
            source,
            Err(InterpretError::UndefinedBehaviour(_) | InterpretError::PoisonObserved(_))
        ));
        f.lower();
        assert_eq!(f.count(Opcode::Assume), 0);
        assert_eq!(f.count(Opcode::VariantWitness), 0);
        assert_eq!(physical(&f.graph, 9), Ok(Outcome::Return(9)));
    }
}

#[test]
fn outer_witness_does_not_assert_a_variant_of_its_nested_payload() {
    let mut f = Fixture::new(2);
    let left = f.variant(0, &[f.arg, f.arg]);
    let right = f.variant(1, &[f.arg, f.arg]);
    let inner = merged_variants(&mut f, [left, right]);
    let outer = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "OuterWitness".into(),
        },
        None,
    );
    f.graph.oracle.record_semantic_aggregate(
        outer,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![], vec![], vec![f.enumeration]],
        },
    );
    f.graph.oracle.record_semantic_enum(outer, vec![0, 1, 2]);
    let value = f.graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[inner],
        2,
        ValueType::Rust(outer),
    );
    let proof = witness(&mut f, value, 2);
    let enumeration = f.enumeration;
    let inner = witnessed_field(&mut f, value, proof, 2, enumeration);
    let index = f.graph.add_node(
        Opcode::Discriminant,
        NodeId::NONE,
        &[inner],
        0,
        ValueType::Rust(f.word),
    );
    f.graph.add_node(
        Opcode::Return,
        f.control,
        &[f.effect, index],
        0,
        ValueType::Control,
    );
    verify_graph(&mut f.graph).unwrap();
    for arg in [0, 1, 53] {
        assert_eq!(
            semantic(&f.graph, arg),
            Ok(SemanticOutcome::Return(Value::Scalar(u64::from(arg != 0))))
        );
    }
    f.lower();
    for arg in [0, 1, 53] {
        assert_eq!(
            physical(&f.graph, arg),
            Ok(Outcome::Return(u64::from(arg != 0)))
        );
    }
}
