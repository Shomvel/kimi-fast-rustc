use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeOracle, TypeShape,
};
use kfrc_son::specialize::{HoleAnswer, RustDefinition, SpecializationOracle};
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, GraphState, Hole, Interpreter, NodeId, Opcode,
    RustSignature, SemanticOutcome, TraitCall, Value, ValueType,
};
use std::collections::HashMap;

fn projection(graph: &mut FunctionGraph, source: NodeId, index: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty)
}
fn types() -> (RecordedTypes, TyId, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        None,
    );
    let boolean = types.intern(TypeShape::Bool, None);
    let parameter = types.intern(
        TypeShape::Param {
            index: 0,
            name: "T".into(),
        },
        None,
    );
    (types, word, boolean, parameter)
}
struct Answers {
    source: RecordedTypes,
    abi: FunctionAbi,
    size: u64,
    symbol: String,
    signed: bool,
    forbid_resolve: bool,
    asked: Vec<Hole>,
    mapped: HashMap<TyId, TyId>,
}
impl SpecializationOracle for Answers {
    fn instantiate_type(&mut self, ty: TyId, into: &mut RecordedTypes) -> Result<TyId, String> {
        if let Some(&mapped) = self.mapped.get(&ty) {
            return Ok(mapped);
        }
        let (shape, fallback_size) = match self.source.shape(ty).clone() {
            TypeShape::Param { .. } => (
                TypeShape::Int {
                    width: IntegerWidth::W32,
                    signed: self.signed,
                },
                4,
            ),
            TypeShape::Bool => (TypeShape::Bool, 1),
            TypeShape::Unit => (TypeShape::Unit, 0),
            TypeShape::PlaceAddress { .. } => {
                return Err("internal address must be remapped by the specializer".into());
            }
            TypeShape::Tuple(fields) => (
                TypeShape::Tuple(
                    fields
                        .iter()
                        .map(|ty| self.instantiate_type(*ty, into))
                        .collect::<Result<_, _>>()?,
                ),
                0,
            ),
            TypeShape::Array { element, length } => (
                TypeShape::Array {
                    element: self.instantiate_type(element, into)?,
                    length,
                },
                0,
            ),
            shape => (shape, 8),
        };
        let size = self
            .source
            .size_align(ty)
            .map_or(fallback_size, |layout| layout.size);
        let mapped = into.intern(
            shape,
            Some(SizeAlign {
                size,
                align: size.max(1),
            }),
        );
        self.mapped.insert(ty, mapped);
        if let Some(aggregate) = self.source.semantic_aggregate(ty).cloned() {
            let fields = aggregate
                .fields
                .iter()
                .map(|ty| self.instantiate_type(*ty, into))
                .collect::<Result<_, _>>()?;
            let variants = aggregate
                .variants
                .iter()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|ty| self.instantiate_type(*ty, into))
                        .collect::<Result<_, _>>()
                })
                .collect::<Result<_, _>>()?;
            into.record_semantic_aggregate(
                mapped,
                kfrc_body::SemanticAggregate { fields, variants },
            );
        }
        if let Some(discriminants) = self.source.semantic_discriminants(ty) {
            into.record_semantic_enum(mapped, discriminants.to_vec());
        }
        if let Some(variants) = self.source.semantic().inhabited_variants(ty) {
            into.record_semantic_inhabited_variants(mapped, variants.to_vec());
        }
        if self.source.semantic().is_ordering(ty) {
            into.record_ordering(mapped);
        }
        if self.source.semantic().is_unsized(ty) {
            into.record_unsized(mapped);
        }
        if self.source.semantic().is_thin_unsized(ty) {
            into.record_thin_unsized(mapped);
        }
        if let Some(field) = self.source.semantic().static_slice_tail(ty) {
            into.record_static_slice_tail(mapped, field);
        }
        Ok(mapped)
    }
    fn function_abi(&mut self) -> Result<FunctionAbi, String> {
        Ok(self.abi.clone())
    }
    fn answer(&mut self, hole: &Hole) -> Result<HoleAnswer, String> {
        self.asked.push(hole.clone());
        Ok(match hole {
            Hole::SizeAlign(_) => HoleAnswer::SizeAlign(SizeAlign {
                size: self.size,
                align: 1,
            }),
            Hole::NeedsDrop(_) => HoleAnswer::NeedsDrop(false),
            Hole::Resolve { .. } => {
                assert!(!self.forbid_resolve, "dead branch queried Resolve");
                HoleAnswer::Resolved {
                    target: CallTarget {
                        symbol: self.symbol.clone(),
                        abi: self.abi.clone(),
                    },
                    identity: self.symbol.clone(),
                }
            }
            Hole::VariantEncoding(_) => return Err("unused encoding must stay cold".into()),
        })
    }
}
fn answers(source: &RecordedTypes, args: usize) -> Answers {
    Answers {
        source: source.clone(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct; args],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        size: 0,
        symbol: "live_target".into(),
        signed: false,
        forbid_resolve: false,
        asked: Vec::new(),
        mapped: HashMap::new(),
    }
}
fn lazy_definition() -> RustDefinition {
    let (types, word, boolean, parameter) = types();
    let mut graph = FunctionGraph::new_rust(
        "choose",
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
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let question = graph.add_hole(Hole::SizeAlign(parameter));
    let hole = graph.add_node(
        Opcode::Hole,
        NodeId::NONE,
        &[],
        question,
        ValueType::Tuple(vec![ValueType::Rust(word), ValueType::Rust(word)]),
    );
    let size = projection(&mut graph, hole, 0, ValueType::Rust(word));
    // Match the production converter: size_of returns into the next MIR
    // block, and definition-level slot promotion introduces a one-edge Phi.
    let jump = graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
    let control = graph.add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
    let size = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[control, size],
        0,
        ValueType::Rust(word),
    );
    let zero = graph.add_constant(0, word);
    let condition = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[size, zero],
        ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let branch = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let yes = projection(&mut graph, branch, 0, ValueType::Control);
    let no = projection(&mut graph, branch, 1, ValueType::Control);
    let answer = graph.add_constant(42, word);
    graph.add_node(
        Opcode::Return,
        yes,
        &[effect, answer],
        0,
        ValueType::Control,
    );
    let resolve = graph.add_hole(Hole::Resolve {
        item: "normal_path".into(),
        args: vec![parameter],
    });
    graph.add_hole(Hole::VariantEncoding(parameter));
    graph.trait_calls.push(TraitCall {
        resolve,
        signature: RustSignature {
            args: vec![],
            ret: word,
        },
    });
    let call = graph.add_node(
        Opcode::TraitCall,
        no,
        &[effect],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    let control = projection(&mut graph, call, 0, ValueType::Control);
    let effect = projection(&mut graph, call, 1, ValueType::Effect);
    let value = projection(&mut graph, call, 2, ValueType::Rust(word));
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    graph.state = GraphState::Sealed;
    RustDefinition::prepare(graph).unwrap()
}
#[test]
fn zst_control_question_prunes_before_copying_or_resolving_dead_arm() {
    let definition = lazy_definition();
    let mut r_oracle = answers(&definition.graph().oracle, 0);
    r_oracle.forbid_resolve = true;
    let r_result = Interpreter::new(definition.graph())
        .run_rust_values(
            &[],
            &mut |_: &str, _: &[Value]| unreachable!(),
            &mut r_oracle,
        )
        .unwrap();
    assert_eq!(r_result, SemanticOutcome::Return(Value::Scalar(42)));
    assert_eq!(r_oracle.asked.len(), 1);
    let mut oracle = answers(&definition.graph().oracle, 0);
    oracle.forbid_resolve = true;
    let specialized = definition.specialize("choose_unit", &mut oracle).unwrap();
    assert_eq!(specialized.stats.holes_answered, 1);
    assert_eq!(specialized.stats.branches_pruned, 1);
    assert!(specialized.stats.nodes_copied < definition.graph().len());
    assert!(specialized.graph.callees.is_empty());
    assert!(specialized.graph.ids().all(|node| !matches!(
        specialized.graph.op(node),
        Opcode::Hole | Opcode::TraitCall | Opcode::If
    )));
    assert_eq!(
        Interpreter::new(&specialized.graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
    // The immutable definition still includes the alternate path for another T.
    let mut nonzero = answers(&definition.graph().oracle, 0);
    nonzero.size = 4;
    let normal = definition.specialize("choose_word", &mut nonzero).unwrap();
    assert_eq!(normal.stats.holes_answered, 2);
    assert_eq!(normal.graph.callees[0].symbol, "live_target");
    assert_eq!(
        Interpreter::new(&normal.graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| Ok(Value::Scalar(91)))
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(91))
    );
}
#[test]
fn static_trait_resolution_distinguishes_equal_size_implementations() {
    let (types, _word, boolean, parameter) = types();
    let mut graph = FunctionGraph::new_rust(
        "greater",
        RustSignature {
            args: vec![parameter, parameter],
            ret: boolean,
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
            ValueType::Rust(parameter),
            ValueType::Rust(parameter),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let left = projection(&mut graph, start, 2, ValueType::Rust(parameter));
    let right = projection(&mut graph, start, 3, ValueType::Rust(parameter));
    let resolve = graph.add_hole(Hole::Resolve {
        item: "PartialOrd::gt".into(),
        args: vec![parameter],
    });
    graph.trait_calls.push(TraitCall {
        resolve,
        signature: RustSignature {
            args: vec![parameter, parameter],
            ret: boolean,
        },
    });
    let call = graph.add_node(
        Opcode::TraitCall,
        control,
        &[effect, left, right],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(boolean),
        ]),
    );
    let control = projection(&mut graph, call, 0, ValueType::Control);
    let effect = projection(&mut graph, call, 1, ValueType::Effect);
    let value = projection(&mut graph, call, 2, ValueType::Rust(boolean));
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    graph.state = GraphState::Sealed;
    let definition = RustDefinition::prepare(graph).unwrap();
    let mut identities = Vec::new();
    for (symbol, signed, expected) in [
        ("u32_gt", false, 1),
        ("i32_gt", true, 0),
        ("custom_gt", false, 0),
    ] {
        let mut oracle = answers(&definition.graph().oracle, 2);
        oracle.symbol = symbol.into();
        oracle.signed = signed;
        oracle.size = 4;
        let result = definition.specialize(symbol, &mut oracle).unwrap();
        assert_eq!(result.stats.holes_answered, 1);
        let HoleAnswer::Resolved { identity, .. } = &result.answers[0].1 else {
            panic!("resolution answer")
        };
        identities.push(identity.clone());
        let outcome = Interpreter::new(&result.graph)
            .run_values(
                &[Value::Scalar(u32::MAX.into()), Value::Scalar(0)],
                &mut |callee: &str, args: &[Value]| {
                    assert_eq!(callee, symbol);
                    let [Value::Scalar(left), Value::Scalar(right)] = args else {
                        panic!("scalar args")
                    };
                    let value = match callee {
                        "u32_gt" => left > right,
                        "i32_gt" => (*left as i32) > (*right as i32),
                        "custom_gt" => false,
                        _ => unreachable!(),
                    };
                    Ok(Value::Scalar(u64::from(value)))
                },
            )
            .unwrap();
        assert_eq!(outcome, SemanticOutcome::Return(Value::Scalar(expected)));
    }
    assert_ne!(identities[0], identities[1]);
    assert_ne!(identities[0], identities[2]);
}

#[test]
fn constructed_variant_prunes_switch_and_remaps_tag_only_witness() {
    use kfrc_body::SemanticAggregate;
    let (mut types, word, _, _) = types();
    let choice = types.intern(
        TypeShape::Adt {
            name: "Choice".into(),
        },
        None,
    );
    types.record_semantic_aggregate(
        choice,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![], vec![word, word]],
        },
    );
    types.record_semantic_enum(choice, vec![0, 1]);
    let mut graph = FunctionGraph::new_rust(
        "chosen",
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
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let answer = graph.add_constant(17, word);
    let poison_payload = graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(word),
    );
    let value = graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[answer, poison_payload],
        1,
        ValueType::Rust(choice),
    );
    graph.variant_switches.push(vec![1]);
    let switch = graph.add_node(
        Opcode::VariantSwitch,
        control,
        &[value],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let some = projection(&mut graph, switch, 0, ValueType::Control);
    let none = projection(&mut graph, switch, 1, ValueType::Control);
    let witness = graph.add_node(
        Opcode::VariantWitness,
        some,
        &[value, some],
        1,
        ValueType::Witness,
    );
    graph.variant_fields.push((1, 0));
    let field = graph.add_node(
        Opcode::VariantField,
        some,
        &[value, witness],
        0,
        ValueType::Rust(word),
    );
    graph.add_node(
        Opcode::Return,
        some,
        &[effect, field],
        0,
        ValueType::Control,
    );
    let zero = graph.add_constant(0, word);
    graph.add_node(Opcode::Return, none, &[effect, zero], 0, ValueType::Control);
    graph.state = GraphState::Sealed;
    let definition = RustDefinition::prepare_with_optimization(graph, false).unwrap();
    let prepared = RustDefinition::prepare(definition.graph().clone()).unwrap();
    assert_eq!(prepared.preparation_stats().branches_folded, 1);
    assert!(
        prepared
            .graph()
            .ids()
            .all(|node| prepared.graph().op(node) != Opcode::VariantSwitch)
    );
    let mut prepared_oracle = answers(&prepared.graph().oracle, 0);
    let prepared_instance = prepared
        .specialize("prepared_chosen", &mut prepared_oracle)
        .unwrap();
    assert_eq!(prepared_instance.stats.branches_pruned, 0);
    assert_eq!(
        Interpreter::new(&prepared_instance.graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(17))
    );
    let mut r = answers(&definition.graph().oracle, 0);
    let r_result = Interpreter::new(definition.graph())
        .run_rust_values(&[], &mut |_: &str, _: &[Value]| unreachable!(), &mut r)
        .unwrap();
    let mut s = answers(&definition.graph().oracle, 0);
    let result = definition.specialize("chosen", &mut s).unwrap();
    assert_eq!(result.stats.branches_pruned, 1);
    assert!(
        result
            .graph
            .ids()
            .all(|node| result.graph.op(node) != Opcode::VariantSwitch)
    );
    let witness = result
        .graph
        .ids()
        .find(|node| result.graph.op(*node) == Opcode::VariantWitness)
        .unwrap();
    assert_eq!(result.graph.value_type(witness), &ValueType::Witness);
    assert!(!result.graph.op(witness).is_fixed());
    let proof = result.graph.input(witness, 1);
    assert_eq!(result.graph.op(proof), Opcode::Projection);
    assert_eq!(result.graph.input(proof, 0), result.graph.start);
    let s_result = Interpreter::new(&result.graph)
        .run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
        .unwrap();
    assert_eq!(r_result, s_result);
    assert_eq!(s_result, SemanticOutcome::Return(Value::Scalar(17)));
}

#[test]
fn specialization_seeds_only_changed_or_type_dependent_nodes_and_users() {
    let (types, word, _, parameter) = types();
    let mut graph = FunctionGraph::new_rust(
        "independent",
        RustSignature {
            args: vec![parameter, word],
            ret: word,
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
            ValueType::Rust(parameter),
            ValueType::Rust(word),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let argument = projection(&mut graph, start, 3, ValueType::Rust(word));
    let one = graph.add_constant(1, word);
    let add = graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[argument, one],
        0,
        ValueType::Rust(word),
    );
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, add],
        0,
        ValueType::Control,
    );
    graph.state = GraphState::Sealed;
    let definition = RustDefinition::prepare(graph).unwrap();
    let mut oracle = answers(&definition.graph().oracle, 2);
    let specialized = definition.specialize("independent", &mut oracle).unwrap();
    assert!(specialized.dirty_nodes.is_empty());
    assert!(specialized.local_slots.is_empty());

    let definition = lazy_definition();
    let mut oracle = answers(&definition.graph().oracle, 0);
    let mut specialized = definition.specialize("zst_seams", &mut oracle).unwrap();
    assert!(!specialized.dirty_nodes.is_empty());
    assert!(specialized.dirty_nodes.iter().all(|node| node.index() < specialized.graph.len() && !specialized.graph.is_dead(*node)));
    kfrc_son::optimize_specialized_seams(
        &mut specialized.graph,
        &specialized.dirty_nodes,
        &specialized.local_slots,
    )
    .unwrap();
    assert_eq!(
        Interpreter::new(&specialized.graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
}

#[test]
fn ignored_return_preserves_exact_semantic_initialization_contract() {
    let (mut types, word, _, _) = types();
    let zst = Some(SizeAlign { size: 0, align: 1 });
    let unit = types.intern(TypeShape::Unit, zst);
    let product = types.intern(TypeShape::Tuple(vec![unit]), zst);
    let empty_array = types.intern(
        TypeShape::Array {
            element: word,
            length: 0,
        },
        zst,
    );
    let unit_array = types.intern(
        TypeShape::Array {
            element: unit,
            length: 4,
        },
        zst,
    );
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "IgnoredEnum".into(),
        },
        zst,
    );
    types.record_semantic_aggregate(
        enumeration,
        kfrc_body::SemanticAggregate {
            fields: vec![],
            variants: vec![vec![unit]],
        },
    );
    types.record_semantic_enum(enumeration, vec![0]);
    types.record_semantic_inhabited_variants(enumeration, vec![0]);
    // Undefined has a structural meaning shared by R and S: zero-leaf
    // products have no poison to observe; an unknown enum variant is poison.
    for (return_ty, poison_expected) in [
        (unit, false),
        (product, false),
        (empty_array, false),
        (unit_array, false),
        (enumeration, true),
    ] {
        let mut graph = FunctionGraph::new_rust(
            "ignored_observation",
            RustSignature {
                args: vec![],
                ret: return_ty,
            },
            types.clone(),
        );
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        graph.start = start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let value = graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(return_ty),
        );
        graph.add_node(
            Opcode::Return,
            control,
            &[effect, value],
            0,
            ValueType::Control,
        );
        graph.state = GraphState::Sealed;
        let definition = RustDefinition::prepare(graph).unwrap();
        let mut r = answers(&definition.graph().oracle, 0);
        let r_result = Interpreter::new(definition.graph()).run_rust_values(
            &[],
            &mut |_: &str, _: &[Value]| unreachable!(),
            &mut r,
        );
        assert_eq!(
            matches!(r_result, Err(kfrc_son::InterpretError::PoisonObserved(_))),
            poison_expected,
            "R return {return_ty:?}: {r_result:?}"
        );
        if !poison_expected {
            assert!(r_result.is_ok());
        }
        let mut s = answers(&definition.graph().oracle, 0);
        s.abi.ret = PassMode::Ignore;
        let mut result = definition
            .specialize("ignored_observation", &mut s)
            .unwrap();
        let returned = result
            .graph
            .ids()
            .find(|node| result.graph.op(*node) == Opcode::Return)
            .unwrap();
        assert_eq!(result.graph.inputs(returned).len(), 2);
        kfrc_son::optimize_specialized_seams(
            &mut result.graph,
            &result.dirty_nodes,
            &result.local_slots,
        )
        .unwrap();
        let s_result = Interpreter::new(&result.graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| unreachable!());
        assert_eq!(
            matches!(s_result, Err(kfrc_son::InterpretError::PoisonObserved(_))),
            poison_expected,
            "S return {return_ty:?}: {s_result:?}"
        );
        if !poison_expected {
            assert_eq!(s_result, r_result);
        }
    }
}

#[test]
fn ignored_call_result_and_argument_preserve_source_values() {
    let (mut types, _, _, _) = types();
    let unit = types.intern(TypeShape::Unit, None);
    let mut graph = FunctionGraph::new_rust(
        "ignored_identity",
        RustSignature {
            args: vec![unit],
            ret: unit,
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
            ValueType::Rust(unit),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let argument = projection(&mut graph, start, 2, ValueType::Rust(unit));
    let resolve = graph.add_hole(Hole::Resolve {
        item: "identity".into(),
        args: vec![unit],
    });
    graph.trait_calls.push(TraitCall {
        resolve,
        signature: RustSignature {
            args: vec![unit],
            ret: unit,
        },
    });
    let call = graph.add_node(
        Opcode::TraitCall,
        control,
        &[effect, argument],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(unit),
        ]),
    );
    let control = projection(&mut graph, call, 0, ValueType::Control);
    let effect = projection(&mut graph, call, 1, ValueType::Effect);
    let value = projection(&mut graph, call, 2, ValueType::Rust(unit));
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    graph.state = GraphState::Sealed;
    let definition = RustDefinition::prepare(graph).unwrap();
    let mut oracle = answers(&definition.graph().oracle, 1);
    oracle.abi.args = vec![PassMode::Ignore];
    oracle.abi.ret = PassMode::Ignore;
    let result = definition
        .specialize("ignored_identity", &mut oracle)
        .unwrap();
    for parent in [Opcode::Start, Opcode::Call] {
        assert!(
            result.graph.ids().any(|node| {
                result.graph.op(node) == Opcode::Projection
                    && result.graph.payload(node) == 2
                    && result.graph.op(result.graph.input(node, 0)) == parent
            }),
            "Ignore ABI must preserve the source {parent:?} value"
        );
    }
    let mut calls = 0;
    let outcome = Interpreter::new(&result.graph)
        .run_values(
            &[Value::Aggregate(vec![])],
            &mut |_: &str, args: &[Value]| {
                calls += 1;
                assert_eq!(args, &[Value::Aggregate(vec![])]);
                Ok(Value::Aggregate(vec![]))
            },
        )
        .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(outcome, SemanticOutcome::Return(Value::Aggregate(vec![])));
    for returned in [Value::Poison, Value::Scalar(7)] {
        let mut r_oracle = answers(&definition.graph().oracle, 1);
        let r = Interpreter::new(definition.graph()).run_rust_values(
            &[Value::Aggregate(vec![])],
            &mut |_: &str, _: &[Value]| Ok(returned.clone()),
            &mut r_oracle,
        );
        let s = Interpreter::new(&result.graph)
            .run_values(&[Value::Aggregate(vec![])], &mut |_: &str, _: &[Value]| {
                Ok(returned.clone())
            });
        match returned {
            Value::Poison => {
                assert!(matches!(
                    r,
                    Err(kfrc_son::InterpretError::PoisonObserved(_))
                ));
                assert!(matches!(
                    s,
                    Err(kfrc_son::InterpretError::PoisonObserved(_))
                ));
            }
            _ => {
                assert!(matches!(r, Err(kfrc_son::InterpretError::Malformed(_))));
                assert_eq!(s, r);
            }
        }
    }
}

#[test]
fn internal_place_address_instantiates_owner_without_a_native_type_query() {
    let (mut types, _, _, parameter) = types();
    let address = types.intern(TypeShape::PlaceAddress { pointee: parameter }, None);
    let mut graph = FunctionGraph::new_rust(
        "internal_address",
        RustSignature {
            args: vec![parameter],
            ret: parameter,
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
            ValueType::Rust(parameter),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let argument = projection(&mut graph, start, 2, ValueType::Rust(parameter));
    let slot = graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(address),
    );
    let store = graph.add_node(
        Opcode::StoreValue,
        control,
        &[effect, slot, argument],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = projection(&mut graph, store, 0, ValueType::Control);
    let effect = projection(&mut graph, store, 1, ValueType::Effect);
    let load = graph.add_node(
        Opcode::LoadValue,
        control,
        &[effect, slot],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(parameter),
        ]),
    );
    let control = projection(&mut graph, load, 0, ValueType::Control);
    let effect = projection(&mut graph, load, 1, ValueType::Effect);
    let value = projection(&mut graph, load, 2, ValueType::Rust(parameter));
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    graph.state = GraphState::Sealed;
    let definition = RustDefinition::prepare_with_optimization(graph, false).unwrap();
    let mut oracle = answers(&definition.graph().oracle, 1);
    let mut result = definition
        .specialize("internal_address", &mut oracle)
        .unwrap();
    assert_eq!(result.local_slots.len(), 1);
    let slot = result.local_slots[0];
    let address = result.graph.rust_type(slot).unwrap();
    let TypeShape::PlaceAddress { pointee } = result.graph.oracle.shape(address) else {
        panic!("lost internal address");
    };
    assert!(matches!(
        result.graph.oracle.shape(*pointee),
        TypeShape::Int {
            width: IntegerWidth::W32,
            signed: false
        }
    ));
    kfrc_son::optimize_specialized_seams(
        &mut result.graph,
        &result.dirty_nodes,
        &result.local_slots,
    )
    .unwrap();
    assert_eq!(
        Interpreter::new(&result.graph)
            .run_values(
                &[Value::Scalar(37)],
                &mut |_: &str, _: &[Value]| unreachable!()
            )
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(37))
    );
}

#[test]
fn definition_control_cleanup_removes_transitively_dead_questions_once() {
    const DEPTH: usize = 6;
    let (types, word, boolean, parameter) = types();
    let mut graph = FunctionGraph::new_rust(
        "independent_control",
        RustSignature {
            args: vec![word],
            ret: word,
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
        ]),
    );
    graph.start = start;
    let mut control = projection(&mut graph, start, 0, ValueType::Control);
    let mut effect = projection(&mut graph, start, 1, ValueType::Effect);
    let argument = projection(&mut graph, start, 2, ValueType::Rust(word));
    // This live independent arithmetic must not be swept by the CFG repair
    // worklist. It is already optimized once before control cleanup starts.
    let mut output = argument;
    for _ in 0..128 {
        output = graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[output, argument],
            0,
            ValueType::Rust(word),
        );
    }
    let yes_value = graph.add_constant(1, boolean);
    let no_value = graph.add_constant(0, boolean);
    let mut condition = yes_value;
    for index in 0..DEPTH {
        let branch = graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        let yes = projection(&mut graph, branch, 0, ValueType::Control);
        let no = projection(&mut graph, branch, 1, ValueType::Control);
        let yes_jump = graph.add_node(Opcode::Jump, yes, &[], 0, ValueType::Control);
        let resolve = graph.add_hole(Hole::Resolve {
            item: format!("dead_{index}"),
            args: vec![parameter],
        });
        let payload = graph.trait_calls.len() as u32;
        graph.trait_calls.push(TraitCall {
            resolve,
            signature: RustSignature {
                args: vec![],
                ret: word,
            },
        });
        let call = graph.add_node(
            Opcode::TraitCall,
            no,
            &[effect],
            payload,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
            ]),
        );
        let after_call = projection(&mut graph, call, 0, ValueType::Control);
        let after_effect = projection(&mut graph, call, 1, ValueType::Effect);
        let no_jump = graph.add_node(Opcode::Jump, after_call, &[], 0, ValueType::Control);
        control = graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[yes_jump, no_jump],
            0,
            ValueType::Control,
        );
        effect = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[control, effect, after_effect],
            0,
            ValueType::Effect,
        );
        let selected = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[control, yes_value, no_value],
            0,
            ValueType::Rust(boolean),
        );
        condition = graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[selected, yes_value],
            ComparePredicate::Equal as u32,
            ValueType::Rust(boolean),
        );
    }
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, output],
        0,
        ValueType::Control,
    );
    graph.state = GraphState::Sealed;
    let baseline = RustDefinition::prepare_with_optimization(graph.clone(), false).unwrap();
    assert_eq!(baseline.preparation_stats().control_simplification_runs, 0);
    let mut baseline_oracle = answers(&baseline.graph().oracle, 1);
    baseline_oracle.forbid_resolve = true;
    let expected = Interpreter::new(baseline.graph())
        .run_rust_values(
            &[Value::Scalar(3)],
            &mut |_: &str, _: &[Value]| unreachable!(),
            &mut baseline_oracle,
        )
        .unwrap();
    assert_eq!(expected, SemanticOutcome::Return(Value::Scalar(387)));
    assert!(baseline_oracle.asked.is_empty());

    let definition = RustDefinition::prepare(graph).unwrap();
    let stats = definition.preparation_stats().clone();
    assert_eq!(stats.branches_folded, DEPTH);
    assert_eq!(stats.predecessor_edges_removed, DEPTH);
    assert_eq!(stats.control_simplification_runs, DEPTH);
    assert!(stats.control_nodes_removed >= DEPTH);
    assert!(stats.control_rule_nodes_visited > 0);
    assert!(
        stats.control_rule_nodes_visited <= 8 * DEPTH,
        "unrelated arithmetic entered CFG rule seeds: {stats:?}"
    );
    assert!(definition.graph().ids().all(|node| !matches!(
        definition.graph().op(node),
        Opcode::If | Opcode::VariantSwitch | Opcode::TraitCall | Opcode::Hole
    )));
    // Metadata remains available for diagnostics; executable consumers vanish.
    assert_eq!(definition.graph().holes.len(), DEPTH);
    let prepared_dump = definition.graph().dump();
    for signed in [false, true] {
        let mut oracle = answers(&definition.graph().oracle, 1);
        oracle.forbid_resolve = true;
        oracle.signed = signed;
        let specialized = definition
            .specialize("independent_control", &mut oracle)
            .unwrap();
        assert_eq!(specialized.stats.branches_pruned, 0);
        assert_eq!(specialized.stats.holes_answered, 0);
        assert_eq!(
            Interpreter::new(&specialized.graph)
                .run_values(
                    &[Value::Scalar(3)],
                    &mut |_: &str, _: &[Value]| unreachable!()
                )
                .unwrap(),
            expected
        );
    }
    assert_eq!(definition.preparation_stats(), &stats);
    assert_eq!(definition.graph().dump(), prepared_dump);
}

#[test]
fn definition_control_cleanup_keeps_unanswered_control_questions() {
    let definition = lazy_definition();
    assert_eq!(definition.preparation_stats().branches_folded, 0);
    assert!(
        definition
            .graph()
            .ids()
            .any(|node| definition.graph().op(node) == Opcode::Hole)
    );
    assert!(
        definition
            .graph()
            .ids()
            .any(|node| definition.graph().op(node) == Opcode::TraitCall)
    );
    let mut zst = answers(&definition.graph().oracle, 0);
    zst.forbid_resolve = true;
    let result = definition.specialize("query_dependent", &mut zst).unwrap();
    assert_eq!(result.stats.branches_pruned, 1);
    assert_eq!(result.stats.holes_answered, 1);
}

#[test]
fn uninhabited_ignored_argument_keeps_an_impossible_entry_without_a_constructor() {
    let (mut types, word, _, _) = types();
    let empty = types.intern(
        TypeShape::Adt {
            name: "Residual".into(),
        },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    types.record_semantic_aggregate(
        empty,
        kfrc_body::SemanticAggregate {
            fields: vec![],
            variants: vec![],
        },
    );
    types.record_semantic_enum(empty, vec![]);
    types.record_semantic_inhabited_variants(empty, vec![]);
    let address = types.intern(TypeShape::PlaceAddress { pointee: empty }, None);
    let mut graph = FunctionGraph::new_rust(
        "from_residual",
        RustSignature {
            args: vec![empty],
            ret: word,
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
            ValueType::Rust(empty),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let argument = projection(&mut graph, start, 2, ValueType::Rust(empty));
    let slot = graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(address),
    );
    let store = graph.add_node(
        Opcode::StoreValue,
        control,
        &[effect, slot, argument],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = projection(&mut graph, store, 0, ValueType::Control);
    graph.add_node(Opcode::Unreachable, control, &[], 0, ValueType::Control);
    graph.state = GraphState::Sealed;
    for optimize in [false, true] {
        let definition =
            RustDefinition::prepare_with_optimization(graph.clone(), optimize).unwrap();
        let mut oracle = answers(&definition.graph().oracle, 1);
        oracle.abi.args = vec![PassMode::Ignore];
        let mut result = definition.specialize("from_residual", &mut oracle).unwrap();
        assert_eq!(result.stats.impossible_boundaries, 1);
        assert!(result.graph.ids().all(|node| matches!(
            result.graph.op(node),
            Opcode::Start | Opcode::Projection | Opcode::Unreachable
        )));
        assert!(matches!(
            Interpreter::new(&result.graph)
                .run_values(&[Value::Poison], &mut |_: &str, _: &[Value]| unreachable!()),
            Err(kfrc_son::InterpretError::PoisonObserved(_))
        ));
        kfrc_son::lower_layout(&mut result.graph).unwrap();
        kfrc_son::verify_graph(&mut result.graph).unwrap();
        assert!(
            result
                .graph
                .ids()
                .any(|node| result.graph.op(node) == Opcode::Unreachable)
        );
    }
}

#[test]
fn uninhabited_call_result_retains_the_call_and_forbids_its_continuation() {
    let (mut types, _, _, _) = types();
    let never = types.intern(TypeShape::Never, Some(SizeAlign { size: 0, align: 1 }));
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let mut graph = FunctionGraph::new_rust(
        "diverges",
        RustSignature {
            args: vec![],
            ret: unit,
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
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let resolve = graph.add_hole(Hole::Resolve {
        item: "diverge".into(),
        args: vec![],
    });
    graph.trait_calls.push(TraitCall {
        resolve,
        signature: RustSignature {
            args: vec![],
            ret: never,
        },
    });
    let call = graph.add_node(
        Opcode::TraitCall,
        control,
        &[effect],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(never),
        ]),
    );
    let control = projection(&mut graph, call, 0, ValueType::Control);
    let effect = projection(&mut graph, call, 1, ValueType::Effect);
    let value = graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(unit),
    );
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    graph.state = GraphState::Sealed;
    for optimize in [false, true] {
        let definition =
            RustDefinition::prepare_with_optimization(graph.clone(), optimize).unwrap();
        let mut oracle = answers(&definition.graph().oracle, 0);
        oracle.abi.ret = PassMode::Ignore;
        let mut result = definition.specialize("diverges", &mut oracle).unwrap();
        assert_eq!(result.stats.impossible_boundaries, 1);
        assert!(
            !result
                .graph
                .ids()
                .any(|node| result.graph.op(node) == Opcode::Return)
        );
        let mut calls = 0;
        assert!(matches!(
            Interpreter::new(&result.graph).run_values(&[], &mut |_: &str, _: &[Value]| {
                calls += 1;
                // A callee cannot normally return a Never value. This
                // deliberately invalid handler must not bypass the source
                // return contract merely because its ABI ignores bits.
                Ok(Value::Poison)
            }),
            Err(kfrc_son::InterpretError::PoisonObserved(_))
        ));
        let mut r_oracle = answers(&definition.graph().oracle, 0);
        assert!(matches!(
            Interpreter::new(definition.graph()).run_rust_values(
                &[],
                &mut |_: &str, _: &[Value]| Ok(Value::Poison),
                &mut r_oracle,
            ),
            Err(kfrc_son::InterpretError::PoisonObserved(_))
        ));
        assert_eq!(calls, 1);
        kfrc_son::lower_layout(&mut result.graph).unwrap();
        kfrc_son::verify_graph(&mut result.graph).unwrap();
        assert_eq!(
            result
                .graph
                .ids()
                .filter(|node| result.graph.op(*node) == Opcode::Call)
                .count(),
            1
        );
    }
}

#[test]
fn require_defined_is_a_control_requirement_without_a_value_result() {
    let (mut types, word, _, _) = types();
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    for (required_ty, poison_expected) in [(unit, false), (word, true)] {
        let mut graph = FunctionGraph::new_rust(
            "require_defined",
            RustSignature {
                args: vec![],
                ret: word,
            },
            types.clone(),
        );
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        graph.start = start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let required = graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(required_ty),
        );
        let requirement = graph.add_node(
            Opcode::RequireDefined,
            control,
            &[effect, required],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        let control = projection(&mut graph, requirement, 0, ValueType::Control);
        let effect = projection(&mut graph, requirement, 1, ValueType::Effect);
        let returned = graph.add_constant(42, word);
        graph.add_node(
            Opcode::Return,
            control,
            &[effect, returned],
            0,
            ValueType::Control,
        );
        graph.state = GraphState::Sealed;
        let definition = RustDefinition::prepare_with_optimization(graph, false).unwrap();
        let mut oracle = answers(&definition.graph().oracle, 0);
        let r = Interpreter::new(definition.graph()).run_rust_values(
            &[],
            &mut |_: &str, _: &[Value]| unreachable!(),
            &mut oracle,
        );
        let mut oracle = answers(&definition.graph().oracle, 0);
        let specialized = definition
            .specialize("require_defined", &mut oracle)
            .unwrap();
        let s = Interpreter::new(&specialized.graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| unreachable!());
        if poison_expected {
            for (graph, outcome) in [(definition.graph(), r), (&specialized.graph, s)] {
                let Err(kfrc_son::InterpretError::PoisonObserved(observer)) = outcome else {
                    panic!("requirement did not consume poison: {outcome:?}");
                };
                assert_eq!(graph.op(observer), Opcode::RequireDefined);
                assert_eq!(
                    graph.value_type(observer),
                    &ValueType::Tuple(vec![ValueType::Control, ValueType::Effect])
                );
            }
        } else {
            assert_eq!(r, Ok(SemanticOutcome::Return(Value::Scalar(42))));
            assert_eq!(s, r);
        }
    }
}

#[test]
fn assume_false_and_poison_are_ub_in_both_r_and_s() {
    let (types, word, boolean, _) = types();
    for predicate in [Some(false), Some(true), None] {
        let mut graph = FunctionGraph::new_rust(
            "assume",
            RustSignature {
                args: vec![],
                ret: word,
            },
            types.clone(),
        );
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        graph.start = start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let condition = match predicate {
            Some(value) => graph.add_constant(u128::from(value), boolean),
            None => graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(boolean),
            ),
        };
        let assumption = graph.add_node(
            Opcode::Assume,
            control,
            &[effect, condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        let control = projection(&mut graph, assumption, 0, ValueType::Control);
        let effect = projection(&mut graph, assumption, 1, ValueType::Effect);
        let value = graph.add_constant(42, word);
        graph.add_node(
            Opcode::Return,
            control,
            &[effect, value],
            0,
            ValueType::Control,
        );
        graph.state = GraphState::Sealed;
        let definition = RustDefinition::prepare_with_optimization(graph, false).unwrap();
        let mut oracle = answers(&definition.graph().oracle, 0);
        let r = Interpreter::new(definition.graph()).run_rust_values(
            &[],
            &mut |_: &str, _: &[Value]| unreachable!(),
            &mut oracle,
        );
        let mut oracle = answers(&definition.graph().oracle, 0);
        let specialized = definition.specialize("assume", &mut oracle).unwrap();
        let s = Interpreter::new(&specialized.graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| unreachable!());
        if predicate == Some(true) {
            assert_eq!(r, Ok(SemanticOutcome::Return(Value::Scalar(42))));
            assert_eq!(s, r);
        } else {
            assert!(matches!(
                r,
                Err(kfrc_son::InterpretError::UndefinedBehaviour(_))
            ));
            assert!(matches!(
                s,
                Err(kfrc_son::InterpretError::UndefinedBehaviour(_))
            ));
        }
    }
}
