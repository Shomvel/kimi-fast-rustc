use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, TyId, TypeShape,
};
use kfrc_son::{
    ComparePredicate, FunctionGraph, GraphLevel, Hole, InterpretError, Interpreter, NodeId, Opcode,
    RustSignature, SemanticOutcome, TraitCall, Value, ValueType, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    enumeration: TyId,
    argument: NodeId,
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
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "OptionalWord".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            enumeration,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![], vec![word]],
            },
        );
        types.record_semantic_enum(enumeration, vec![0, 1]);
        let mut graph = FunctionGraph::new(
            "refine",
            FunctionAbi {
                args: vec![PassMode::Direct],
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
        let argument = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(enumeration),
        );
        Self {
            graph,
            word,
            enumeration,
            argument,
            control,
            effect,
        }
    }
    fn switch(&mut self, value: NodeId) -> (NodeId, NodeId) {
        let payload = self.graph.variant_switches.len() as u32;
        self.graph.variant_switches.push(vec![1]);
        let branch = self.graph.add_node(
            Opcode::VariantSwitch,
            self.control,
            &[value],
            payload,
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
    fn witness(&mut self, value: NodeId, variant: u32, anchor: NodeId, proof: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::VariantWitness,
            anchor,
            &[value, proof],
            variant,
            ValueType::Witness,
        )
    }
    fn witness_field(&mut self, value: NodeId, witness: NodeId) -> NodeId {
        let field = self.field(value);
        self.graph.set_inputs(field, &[value, witness]);
        field
    }
    fn assume_variant(&mut self, value: NodeId, variant: u32) -> NodeId {
        let tag = self.graph.add_node(
            Opcode::VariantOf,
            NodeId::NONE,
            &[value],
            0,
            ValueType::Rust(self.word),
        );
        let expected = self.graph.add_constant(u128::from(variant), self.word);
        let boolean = self.graph.oracle.intern(TypeShape::Bool, None);
        let condition = self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[tag, expected],
            ComparePredicate::Equal as u32,
            ValueType::Rust(boolean),
        );
        let assume = self.graph.add_node(
            Opcode::Assume,
            self.control,
            &[self.effect, condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[assume],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[assume],
            1,
            ValueType::Effect,
        );
        assume
    }
    fn field(&mut self, value: NodeId) -> NodeId {
        let payload = self.graph.intern_variant_field(1, 0);
        self.graph.add_node(
            Opcode::VariantField,
            NodeId::NONE,
            &[value],
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
    fn run(
        &mut self,
        variant: u32,
        payload: Option<u64>,
    ) -> Result<SemanticOutcome, InterpretError> {
        verify_graph(&mut self.graph).unwrap();
        Interpreter::new(&self.graph).run_values(
            &[Value::Variant {
                variant,
                fields: payload.into_iter().map(Value::Scalar).collect(),
            }],
            &mut |_: &str, _: &[Value]| unreachable!(),
        )
    }
}

#[test]
fn proven_variant_and_unique_default_execute_only_under_their_edges() {
    let mut f = Fixture::new();
    let (some, none) = f.switch(f.argument);
    let refined = f.witness(f.argument, 1, some, some);
    let value = f.witness_field(f.argument, refined);
    f.ret(some, value);
    f.witness(f.argument, 0, none, none);
    let zero = f.graph.add_constant(0, f.word);
    f.ret(none, zero);
    assert_eq!(
        f.run(1, Some(37)),
        Ok(SemanticOutcome::Return(Value::Scalar(37)))
    );
    assert_eq!(
        f.run(0, None),
        Ok(SemanticOutcome::Return(Value::Scalar(0)))
    );
}

#[test]
fn proven_fact_rejects_wrong_variant_wrong_value_and_hoisted_anchor() {
    for error in 0..3 {
        let mut f = Fixture::new();
        let (some, none) = f.switch(f.argument);
        let source = if error == 1 {
            f.graph.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.enumeration),
            )
        } else {
            f.argument
        };
        let variant = if error == 0 { 0 } else { 1 };
        let anchor = if error == 2 { f.control } else { some };
        f.witness(source, variant, anchor, some);
        let zero = f.graph.add_constant(0, f.word);
        f.ret(some, zero);
        f.ret(none, zero);
        assert!(
            verify_graph(&mut f.graph).is_err(),
            "invalid proof case {error}"
        );
    }
}

#[test]
fn witnessed_field_cannot_escape_through_a_merge_without_a_phi() {
    let mut f = Fixture::new();
    let (some, none) = f.switch(f.argument);
    let refined = f.witness(f.argument, 1, some, some);
    let value = f.witness_field(f.argument, refined);
    let left = f
        .graph
        .add_node(Opcode::Jump, some, &[], 0, ValueType::Control);
    let right = f
        .graph
        .add_node(Opcode::Jump, none, &[], 0, ValueType::Control);
    let merge = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[left, right],
        0,
        ValueType::Control,
    );
    f.ret(merge, value);
    assert!(
        verify_graph(&mut f.graph).is_err(),
        "a branch witness must not justify a field at an unconditional merge"
    );
}

#[test]
fn assumption_executes_even_without_a_business_result() {
    let mut f = Fixture::new();
    f.assume_variant(f.argument, 1);
    let zero = f.graph.add_constant(0, f.word);
    f.ret(f.control, zero);
    assert_eq!(
        f.run(1, Some(7)),
        Ok(SemanticOutcome::Return(Value::Scalar(0)))
    );
    assert!(matches!(
        f.run(0, None),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    kfrc_son::optimize_specialized_graph(&mut f.graph).unwrap();
    assert!(matches!(
        f.run(0, None),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
}

#[test]
fn tag_witness_and_switch_do_not_observe_unread_payload_poison() {
    let mut f = Fixture::new();
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    let value = f.graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[poison],
        1,
        ValueType::Rust(f.enumeration),
    );
    let (some, none) = f.switch(value);
    f.witness(value, 1, some, some);
    let zero = f.graph.add_constant(0, f.word);
    f.ret(some, zero);
    f.ret(none, zero);
    assert_eq!(
        f.run(0, None),
        Ok(SemanticOutcome::Return(Value::Scalar(0)))
    );
}

#[test]
fn speculative_variant_field_still_produces_poison_without_immediate_ub() {
    let mut f = Fixture::new();
    let inactive = f.graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.enumeration),
    );
    f.field(inactive);
    let zero = f.graph.add_constant(0, f.word);
    f.ret(f.control, zero);
    assert_eq!(
        f.run(0, None),
        Ok(SemanticOutcome::Return(Value::Scalar(0)))
    );
}

#[test]
fn assumption_witness_cannot_be_used_before_its_continuation() {
    let mut f = Fixture::new();
    let before = f.control;
    f.assume_variant(f.argument, 1);
    let witness = f.witness(f.argument, 1, before, f.control);
    let value = f.witness_field(f.argument, witness);
    f.ret(f.control, value);
    assert!(verify_graph(&mut f.graph).is_err());
}

#[test]
fn assumption_can_provide_a_scoped_variant_witness() {
    let mut f = Fixture::new();
    f.assume_variant(f.argument, 1);
    let witness = f.witness(f.argument, 1, f.control, f.control);
    let value = f.witness_field(f.argument, witness);
    f.ret(f.control, value);
    assert_eq!(
        f.run(1, Some(42)),
        Ok(SemanticOutcome::Return(Value::Scalar(42)))
    );
    assert!(matches!(
        f.run(0, None),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
}

#[test]
fn r_signature_and_static_trait_resolution_are_checked_without_an_abi() {
    let mut types = RecordedTypes::new();
    let parameter = types.intern(
        TypeShape::Param {
            index: 0,
            name: "T".into(),
        },
        None,
    );
    let signature = RustSignature {
        args: vec![parameter],
        ret: parameter,
    };
    let mut graph = FunctionGraph::new_rust("generic", signature.clone(), types);
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(parameter),
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
        ValueType::Rust(parameter),
    );
    let resolve = graph.add_hole(Hole::Resolve {
        item: "Identity::apply".into(),
        args: vec![parameter],
    });
    graph.trait_calls.push(TraitCall { resolve, signature });
    let call = graph.add_node(
        Opcode::TraitCall,
        control,
        &[effect, argument],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(parameter),
        ]),
    );
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        1,
        ValueType::Effect,
    );
    let value = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        2,
        ValueType::Rust(parameter),
    );
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    assert!(graph.abi.args.is_empty());
    verify_graph(&mut graph).unwrap();
    let mut answers = IdentityAnswers {
        source: graph.oracle.clone(),
        asked: vec![],
        wrong_kind: false,
    };
    let outcome = Interpreter::new(&graph)
        .run_rust_values(
            &[Value::Scalar(37)],
            &mut |symbol: &str, arguments: &[Value]| {
                assert_eq!(symbol, "identity_u64");
                Ok(arguments[0].clone())
            },
            &mut answers,
        )
        .unwrap();
    assert_eq!(outcome, SemanticOutcome::Return(Value::Scalar(37)));
    assert_eq!(answers.asked, [graph.holes[resolve as usize].clone()]);
    answers.wrong_kind = true;
    assert!(matches!(
        Interpreter::new(&graph).run_rust_values(
            &[Value::Scalar(37)],
            &mut |_: &str, _: &[Value]| unreachable!(),
            &mut answers
        ),
        Err(InterpretError::Malformed(_))
    ));
    graph.holes[resolve as usize] = Hole::NeedsDrop(parameter);
    assert!(verify_graph(&mut graph).is_err());
    graph.holes[resolve as usize] = Hole::Resolve {
        item: "Identity::apply".into(),
        args: vec![parameter],
    };
    graph.level = GraphLevel::Specialized;
    graph.rust_signature = None;
    assert!(verify_graph(&mut graph).is_err());
}

#[test]
fn r_size_align_is_an_explicit_two_result_usize_question() {
    let mut types = RecordedTypes::new();
    let parameter = types.intern(
        TypeShape::Param {
            index: 0,
            name: "T".into(),
        },
        None,
    );
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        None,
    );
    let mut graph = FunctionGraph::new_rust(
        "size",
        RustSignature {
            args: vec![],
            ret: word,
        },
        types,
    );
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
    let question = graph.add_hole(Hole::SizeAlign(parameter));
    let hole = graph.add_node(
        Opcode::Hole,
        NodeId::NONE,
        &[],
        question,
        ValueType::Tuple(vec![ValueType::Rust(word), ValueType::Rust(word)]),
    );
    let size = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[hole],
        0,
        ValueType::Rust(word),
    );
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, size],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    graph.set_value_type(hole, ValueType::Rust(word));
    assert!(verify_graph(&mut graph).is_err());
}

struct IdentityAnswers {
    source: RecordedTypes,
    asked: Vec<Hole>,
    wrong_kind: bool,
}
impl kfrc_son::specialize::SpecializationOracle for IdentityAnswers {
    fn instantiate_type(&mut self, ty: TyId, into: &mut RecordedTypes) -> Result<TyId, String> {
        let shape = match self.source.semantic().shape(ty) {
            TypeShape::Param { .. } => TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            shape => shape.clone(),
        };
        Ok(into.intern(shape, None))
    }
    fn answer(&mut self, hole: &Hole) -> Result<kfrc_son::specialize::HoleAnswer, String> {
        use kfrc_son::specialize::HoleAnswer;
        self.asked.push(hole.clone());
        if self.wrong_kind {
            return Ok(HoleAnswer::NeedsDrop(false));
        }
        Ok(HoleAnswer::Resolved {
            target: kfrc_son::CallTarget {
                symbol: "identity_u64".into(),
                abi: FunctionAbi {
                    args: vec![],
                    ret: PassMode::Ignore,
                    c_variadic: false,
                },
            },
            identity: "identity_u64".into(),
        })
    }
    fn function_abi(&mut self) -> Result<FunctionAbi, String> {
        Err("R execution must not query an ABI".into())
    }
}

// The old L-only VariantIndex Refine is deliberately gone. A logical tag
// predicate is an S assumption; published L must not keep diagnostic checks.
#[test]
fn unused_assumptions_execute_in_s_and_are_rejected_in_published_l() {
    for poison in [false, true] {
        let mut f = Fixture::new();
        let boolean = f.graph.oracle.intern(TypeShape::Bool, None);
        let predicate = if poison {
            f.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(boolean),
            )
        } else {
            f.graph.add_constant(0, boolean)
        };
        let assume = f.graph.add_node(
            Opcode::Assume,
            f.control,
            &[f.effect, predicate],
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
        let zero = f.graph.add_constant(0, f.word);
        f.ret(f.control, zero);
        assert!(matches!(
            f.run(0, None),
            Err(InterpretError::UndefinedBehaviour(_)) | Err(InterpretError::PoisonObserved(_))
        ));
        f.graph.level = GraphLevel::Layout;
        assert!(verify_graph(&mut f.graph).is_err());
    }
}

#[test]
fn assumption_requires_a_boolean_and_has_no_business_result() {
    let mut f = Fixture::new();
    let not_bool = f.graph.add_constant(1, f.word);
    f.graph.add_node(
        Opcode::Assume,
        f.control,
        &[f.effect, not_bool],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    f.ret(f.control, not_bool);
    assert!(verify_graph(&mut f.graph).is_err());

    let mut f = Fixture::new();
    let assumption = f.assume_variant(f.argument, 1);
    f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[assumption],
        2,
        ValueType::Rust(f.enumeration),
    );
    let zero = f.graph.add_constant(0, f.word);
    f.ret(f.control, zero);
    assert!(verify_graph(&mut f.graph).is_err());
}
