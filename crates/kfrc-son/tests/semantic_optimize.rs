use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, SizeAlign, TyId,
    TypeShape,
};
use kfrc_son::interpret::{SemanticOutcome, Value};
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId,
    Opcode, ValueType, optimize_graph, optimize_specialized_graph, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    int: TyId,
    pair: TyId,
    control: NodeId,
    effect: NodeId,
    args: Vec<NodeId>,
}

impl Fixture {
    fn new(arguments: usize) -> Self {
        let mut types = RecordedTypes::new();
        let int = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        // Semantic optimization does not require a tuple's physical layout.
        let pair = types.intern(TypeShape::Tuple(vec![int, int]), None);
        let mut graph = FunctionGraph::new(
            "semantic_optimization",
            FunctionAbi {
                args: vec![PassMode::Direct; arguments],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(std::iter::repeat_n(ValueType::Rust(int), arguments));
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(outputs),
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
        let args = (0..arguments)
            .map(|i| {
                graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[start],
                    i as u32 + 2,
                    ValueType::Rust(int),
                )
            })
            .collect();
        Self {
            graph,
            int,
            pair,
            control,
            effect,
            args,
        }
    }

    fn value(&mut self, op: Opcode, inputs: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, inputs, payload, ValueType::Rust(ty))
    }

    fn finish(&mut self, result: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, result],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
    }

    fn run(&self, args: &[u64]) -> Result<SemanticOutcome, InterpretError> {
        let args: Vec<_> = args.iter().copied().map(Value::Scalar).collect();
        Interpreter::new(&self.graph).run_values(&args, &mut |name: &str, _: &[Value]| {
            Err(InterpretError::UnknownCallee(name.to_owned()))
        })
    }
}

#[test]
fn updates_preserve_old_aggregate_snapshots_without_layout() {
    let mut f = Fixture::new(2);
    let original = f.value(Opcode::Aggregate, &f.args.clone(), 0, f.pair);
    let updated = f.value(Opcode::InsertField, &[original, f.args[1]], 0, f.pair);
    let original_first = f.value(Opcode::Field, &[original], 0, f.int);
    let updated_first = f.value(Opcode::Field, &[updated], 0, f.int);
    let updated_second = f.value(Opcode::Field, &[updated], 1, f.int);
    let left = f.value(Opcode::Mul, &[original_first, updated_first], 0, f.int);
    let result = f.value(Opcode::Add, &[left, updated_second], 0, f.int);
    f.finish(result);
    let mut expected = Vec::new();
    let mut seed = 0x916e_6a8d_f04b_7721u64;
    for _ in 0..128 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let args = [seed, seed.rotate_left(29)];
        let value = args[0].wrapping_mul(args[1]).wrapping_add(args[1]);
        assert_eq!(
            f.run(&args).unwrap(),
            SemanticOutcome::Return(Value::Scalar(value))
        );
        expected.push((args, value));
    }
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert!(!f.graph.ids().any(|id| matches!(
        f.graph.op(id),
        Opcode::Aggregate | Opcode::Field | Opcode::InsertField
    )));
    for (args, value) in expected {
        assert_eq!(
            f.run(&args).unwrap(),
            SemanticOutcome::Return(Value::Scalar(value))
        );
    }
}

#[test]
fn reading_initialized_field_does_not_observe_sibling_poison() {
    for selected in [0, 1] {
        let mut f = Fixture::new(0);
        let poison = f.value(Opcode::Undefined, &[], 0, f.int);
        let answer = f.graph.add_constant(42, f.int);
        let pair = f.value(Opcode::Aggregate, &[poison, answer], 0, f.pair);
        let result = f.value(Opcode::Field, &[pair], selected, f.int);
        f.finish(result);
        for optimized in [false, true] {
            if optimized {
                optimize_graph(&mut f.graph);
                verify_graph(&mut f.graph).unwrap();
            }
            match selected {
                0 => assert!(matches!(f.run(&[]), Err(InterpretError::PoisonObserved(_)))),
                _ => assert_eq!(
                    f.run(&[]).unwrap(),
                    SemanticOutcome::Return(Value::Scalar(42))
                ),
            }
        }
    }
}

#[test]
fn inserting_one_field_initializes_only_that_leaf() {
    for selected in [0, 1] {
        let mut f = Fixture::new(1);
        let poison = f.value(Opcode::Undefined, &[], 0, f.pair);
        let partial = f.value(Opcode::InsertField, &[poison, f.args[0]], 1, f.pair);
        let result = f.value(Opcode::Field, &[partial], selected, f.int);
        f.finish(result);
        for optimized in [false, true] {
            if optimized {
                optimize_graph(&mut f.graph);
                verify_graph(&mut f.graph).unwrap();
            }
            match selected {
                0 => assert!(matches!(
                    f.run(&[91]),
                    Err(InterpretError::PoisonObserved(_))
                )),
                _ => assert_eq!(
                    f.run(&[91]).unwrap(),
                    SemanticOutcome::Return(Value::Scalar(91))
                ),
            }
        }
    }
}

fn enum_type(f: &mut Fixture) -> TyId {
    let ty = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "Choice".into(),
        },
        None,
    );
    f.graph.oracle.record_semantic_aggregate(
        ty,
        SemanticAggregate {
            fields: Vec::new(),
            variants: vec![vec![f.int], vec![f.int]],
        },
    );
    f.graph.oracle.record_semantic_enum(ty, vec![11, 23]);
    ty
}

#[test]
fn logical_variants_and_rust_discriminants_do_not_depend_on_tag_encoding() {
    for (query, expected) in [(Opcode::VariantOf, 1), (Opcode::Discriminant, 23)] {
        let mut f = Fixture::new(0);
        let ty = enum_type(&mut f);
        let poison = f.value(Opcode::Undefined, &[], 0, f.int);
        let value = f.value(Opcode::MakeVariant, &[poison], 1, ty);
        let result = f.value(query, &[value], 0, f.int);
        f.finish(result);
        let before = f.run(&[]).unwrap();
        assert_eq!(before, SemanticOutcome::Return(Value::Scalar(expected)));
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(f.run(&[]).unwrap(), before);
        assert!(!f.graph.ids().any(|id| f.graph.op(id) == query));
    }
}

#[test]
fn an_inactive_variant_field_stays_poison_after_folding() {
    for selected in [0, 1] {
        let mut f = Fixture::new(0);
        let ty = enum_type(&mut f);
        let payload = f.graph.add_constant(9, f.int);
        let value = f.value(Opcode::MakeVariant, &[payload], 1, ty);
        let field = f.graph.intern_variant_field(selected, 0);
        let result = f.value(Opcode::VariantField, &[value], field, f.int);
        f.finish(result);
        for optimized in [false, true] {
            if optimized {
                optimize_graph(&mut f.graph);
                verify_graph(&mut f.graph).unwrap();
            }
            match selected {
                0 => assert!(matches!(f.run(&[]), Err(InterpretError::PoisonObserved(_)))),
                _ => assert_eq!(
                    f.run(&[]).unwrap(),
                    SemanticOutcome::Return(Value::Scalar(9))
                ),
            }
        }
    }
}

#[test]
fn repeated_pure_expressions_share_values_but_calls_keep_their_effects() {
    let mut f = Fixture::new(2);
    let sum_a = f.value(Opcode::Add, &f.args.clone(), 0, f.int);
    let sum_b = f.value(Opcode::Add, &f.args.clone(), 0, f.int);
    let target = f.graph.add_callee(CallTarget {
        symbol: "next".into(),
        abi: FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    let mut results = Vec::new();
    for _ in 0..2 {
        let call = f.graph.add_node(
            Opcode::Call,
            f.control,
            &[f.effect],
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(f.int),
            ]),
        );
        f.control = f.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            0,
            ValueType::Control,
        );
        f.effect = f.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            1,
            ValueType::Effect,
        );
        results.push(f.value(Opcode::Projection, &[call], 2, f.int));
    }
    let calls = f.value(Opcode::Add, &results, 0, f.int);
    let pure = f.value(Opcode::Mul, &[sum_a, sum_b], 0, f.int);
    let result = f.value(Opcode::Add, &[calls, pure], 0, f.int);
    f.finish(result);
    let before = f
        .graph
        .ids()
        .filter(|&id| f.graph.op(id) == Opcode::Add)
        .count();
    for optimized in [false, true] {
        if optimized {
            optimize_graph(&mut f.graph);
            verify_graph(&mut f.graph).unwrap();
            assert_eq!(
                f.graph
                    .ids()
                    .filter(|&id| f.graph.op(id) == Opcode::Add)
                    .count(),
                before - 1
            );
        }
        let mut count = 0;
        let outcome = Interpreter::new(&f.graph)
            .run_values(
                &[Value::Scalar(3), Value::Scalar(5)],
                &mut |name: &str, _: &[Value]| {
                    assert_eq!(name, "next");
                    count += 1;
                    Ok(Value::Scalar(count))
                },
            )
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(outcome, SemanticOutcome::Return(Value::Scalar(67)));
    }
}

fn call(
    f: &mut Fixture,
    symbol: &str,
    control: NodeId,
    effect: NodeId,
) -> (NodeId, NodeId, NodeId) {
    let target = f.graph.add_callee(CallTarget {
        symbol: symbol.into(),
        abi: FunctionAbi {
            args: Vec::new(),
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    let call = f.graph.add_node(
        Opcode::Call,
        control,
        &[effect],
        target,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(f.int),
        ]),
    );
    let control = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        0,
        ValueType::Control,
    );
    let effect = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        1,
        ValueType::Effect,
    );
    let value = f.value(Opcode::Projection, &[call], 2, f.int);
    (control, effect, value)
}

#[test]
fn gvn_representative_placement_follows_fixed_inputs_not_creation_order() {
    let mut f = Fixture::new(1);
    // This expression is allocated before the call that will define its input.
    // Its valid placement is after the call, regardless of its smaller NodeId.
    let first = f.value(Opcode::Add, &[f.args[0], f.args[0]], 0, f.int);
    let (control, effect) = (f.control, f.effect);
    let (control, effect, value) = call(&mut f, "later_definition", control, effect);
    f.graph.set_input(first, 0, value);
    let second = f.value(Opcode::Add, &[value, f.args[0]], 0, f.int);
    let result = f.value(Opcode::Add, &[first, second], 0, f.int);
    f.control = control;
    f.effect = effect;
    f.finish(result);
    for optimized in [false, true] {
        if optimized {
            let stats = optimize_graph(&mut f.graph);
            assert_eq!(stats.values_merged, 1);
            verify_graph(&mut f.graph).unwrap();
        }
        let mut calls = 0;
        let outcome = Interpreter::new(&f.graph)
            .run_values(&[Value::Scalar(5)], &mut |name: &str, _: &[Value]| {
                assert_eq!(name, "later_definition");
                calls += 1;
                Ok(Value::Scalar(16))
            })
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(outcome, SemanticOutcome::Return(Value::Scalar(42)));
    }
}

#[test]
fn enum_folding_removes_only_the_inactive_branch_and_its_merge_slots() {
    for variant in [0, 1] {
        let mut f = Fixture::new(0);
        let ty = enum_type(&mut f);
        let boolean = f
            .graph
            .oracle
            .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let payload = f.value(Opcode::Undefined, &[], 0, f.int);
        let enumeration = f.value(Opcode::MakeVariant, &[payload], variant, ty);
        let discriminant = f.value(Opcode::Discriminant, &[enumeration], 0, f.int);
        let expected_tag = f.graph.add_constant(23, f.int);
        let condition = f.value(
            Opcode::Compare,
            &[discriminant, expected_tag],
            ComparePredicate::Equal as u32,
            boolean,
        );
        let branch = f.graph.add_node(
            Opcode::If,
            f.control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        let mut jumps = Vec::new();
        let mut values = Vec::new();
        let mut effects = Vec::new();
        for (index, symbol) in ["right", "left"].into_iter().enumerate() {
            let control = f.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                index as u32,
                ValueType::Control,
            );
            let initial_effect = f.effect;
            let (control, effect, value) = call(&mut f, symbol, control, initial_effect);
            let jump = f
                .graph
                .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
            jumps.push(jump);
            values.push(value);
            effects.push(effect);
        }
        let region = f
            .graph
            .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
        let result = f.value(Opcode::Phi, &[region, values[0], values[1]], 0, f.int);
        f.effect = f.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, effects[0], effects[1]],
            0,
            ValueType::Effect,
        );
        f.control = region;
        f.finish(result);
        for optimized in [false, true] {
            if optimized {
                optimize_specialized_graph(&mut f.graph).unwrap();
                verify_graph(&mut f.graph).unwrap();
                assert!(!f.graph.ids().any(|id| f.graph.op(id) == Opcode::If));
                assert_eq!(
                    f.graph
                        .ids()
                        .filter(|&id| f.graph.op(id) == Opcode::Call)
                        .count(),
                    1
                );
                for phi in f.graph.ids().filter(|&id| f.graph.op(id) == Opcode::Phi) {
                    let merge = f.graph.input(phi, 0);
                    assert_eq!(f.graph.inputs(phi).len(), f.graph.inputs(merge).len() + 1);
                }
            }
            let mut log = Vec::new();
            let outcome = Interpreter::new(&f.graph)
                .run_values(&[], &mut |name: &str, _: &[Value]| {
                    log.push(name.to_owned());
                    Ok(Value::Scalar(if name == "right" { 23 } else { 11 }))
                })
                .unwrap();
            let (symbol, value) = if variant == 1 {
                ("right", 23)
            } else {
                ("left", 11)
            };
            assert_eq!(log, [symbol]);
            assert_eq!(outcome, SemanticOutcome::Return(Value::Scalar(value)));
        }
    }
}

#[test]
fn repeated_array_projection_preserves_poison_and_stays_compact() {
    for poison in [false, true] {
        let mut f = Fixture::new(1);
        let array = f.graph.oracle.intern(
            TypeShape::Array {
                element: f.int,
                length: 4096,
            },
            None,
        );
        let element = if poison {
            f.value(Opcode::Undefined, &[], 0, f.int)
        } else {
            f.args[0]
        };
        let repeated = f.value(Opcode::Repeat, &[element], 0, array);
        let selected = f.value(Opcode::Field, &[repeated], 4095, f.int);
        f.finish(selected);
        assert!(f.graph.len() < 20);
        let before = f.run(&[37]);
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert!(
            !f.graph
                .ids()
                .any(|n| matches!(f.graph.op(n), Opcode::Field | Opcode::Repeat))
        );
        let after = f.run(&[37]);
        if poison {
            assert!(matches!(before, Err(InterpretError::PoisonObserved(_))));
            assert!(matches!(after, Err(InterpretError::PoisonObserved(_))));
        } else {
            assert_eq!(before.unwrap(), SemanticOutcome::Return(Value::Scalar(37)));
            assert_eq!(after.unwrap(), SemanticOutcome::Return(Value::Scalar(37)));
        }
    }
}

#[test]
fn repeat_value_numbering_keeps_distinct_array_lengths() {
    let mut f = Fixture::new(1);
    let short = f.graph.oracle.intern(
        TypeShape::Array {
            element: f.int,
            length: 4,
        },
        None,
    );
    let long = f.graph.oracle.intern(
        TypeShape::Array {
            element: f.int,
            length: 5,
        },
        None,
    );
    let tuple = f
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![short, short, long]), None);
    let first = f.value(Opcode::Repeat, &[f.args[0]], 0, short);
    let duplicate = f.value(Opcode::Repeat, &[f.args[0]], 0, short);
    let different = f.value(Opcode::Repeat, &[f.args[0]], 0, long);
    let result = f.value(Opcode::Aggregate, &[first, duplicate, different], 0, tuple);
    f.finish(result);
    let before = f.run(&[9]).unwrap();
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(
        f.graph
            .ids()
            .filter(|&n| f.graph.op(n) == Opcode::Repeat)
            .count(),
        2
    );
    assert_eq!(f.run(&[9]).unwrap(), before);
}
fn pointer_argument(f: &mut Fixture, ty: TyId) {
    let mut types = vec![ValueType::Control, ValueType::Effect];
    types.extend(std::iter::repeat_n(ValueType::Rust(ty), f.args.len()));
    f.graph
        .set_value_type(f.graph.start, ValueType::Tuple(types));
    for &arg in &f.args {
        f.graph.set_value_type(arg, ValueType::Rust(ty));
    }
}

#[test]
fn pointer_retyping_roundtrips_preserve_values_and_poison() {
    let mut f = Fixture::new(1);
    let raw = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: kfrc_body::Mutability::Not,
            pointee: f.int,
        },
        None,
    );
    let address = f
        .graph
        .oracle
        .intern(TypeShape::PlaceAddress { pointee: f.int }, None);
    pointer_argument(&mut f, raw);
    let retyped = f.value(Opcode::PointerCast, &[f.args[0]], 0, address);
    let restored = f.value(Opcode::PointerCast, &[retyped], 0, raw);
    let redundant = f.value(Opcode::PointerCast, &[restored], 0, raw);
    f.finish(redundant);
    for optimized in [false, true] {
        if optimized {
            optimize_graph(&mut f.graph);
            verify_graph(&mut f.graph).unwrap();
            assert!(!f.graph.ids().any(|n| f.graph.op(n) == Opcode::PointerCast));
        }
        for value in [
            Value::Scalar(0),
            Value::Scalar(0x1234),
            Value::Scalar(u64::MAX),
            Value::Poison,
        ] {
            let result = Interpreter::new(&f.graph)
                .run_values(&[value.clone()], &mut |name: &str, _: &[Value]| {
                    Err(InterpretError::UnknownCallee(name.into()))
                });
            if value == Value::Poison {
                assert!(matches!(result, Err(InterpretError::PoisonObserved(_))));
            } else {
                assert_eq!(result.unwrap(), SemanticOutcome::Return(value));
            }
        }
    }
}

#[test]
fn rebuilding_a_wide_pointer_preserves_partial_poison_and_source_identity() {
    for same_source in [false, true] {
        for data_only in [false, true] {
            let mut f = Fixture::new(2);
            let raw = f.graph.oracle.intern(
                TypeShape::RawPointer {
                    mutability: kfrc_body::Mutability::Not,
                    pointee: f.int,
                },
                None,
            );
            let slice = f
                .graph
                .oracle
                .intern(TypeShape::Slice { element: f.int }, None);
            let address = f
                .graph
                .oracle
                .intern(TypeShape::PlaceAddress { pointee: slice }, None);
            let wide = f.graph.oracle.intern(
                TypeShape::RawPointer {
                    mutability: kfrc_body::Mutability::Not,
                    pointee: slice,
                },
                None,
            );
            f.graph.oracle.record_semantic_aggregate(
                wide,
                SemanticAggregate {
                    fields: vec![raw, f.int],
                    variants: vec![],
                },
            );
            pointer_argument(&mut f, wide);
            let data = f.value(Opcode::Field, &[f.args[0]], 0, raw);
            let retyped = f.value(Opcode::PointerCast, &[data], 0, address);
            let data = f.value(Opcode::PointerCast, &[retyped], 0, raw);
            let metadata = if same_source {
                f.value(Opcode::Field, &[f.args[0]], 1, f.int)
            } else {
                f.value(Opcode::Field, &[f.args[1]], 1, f.int)
            };
            let rebuilt = f.value(Opcode::WidePtr, &[data, metadata], 0, wide);
            let result = if data_only {
                f.value(Opcode::Field, &[rebuilt], 0, raw)
            } else {
                rebuilt
            };
            f.finish(result);
            let original = f.graph.clone();
            optimize_graph(&mut f.graph);
            verify_graph(&mut f.graph).unwrap();
            assert_eq!(
                f.graph.ids().any(|n| f.graph.op(n) == Opcode::WidePtr),
                !same_source && !data_only
            );
            for (data, metadata) in [
                (Value::Scalar(0x1200), Value::Scalar(7)),
                (Value::Scalar(0x1200), Value::Poison),
                (Value::Poison, Value::Scalar(7)),
            ] {
                let input = Value::WidePointer {
                    data: Box::new(data),
                    metadata: Box::new(metadata),
                };
                let run = |graph: &FunctionGraph| {
                    Interpreter::new(graph).run_values(
                        &[
                            input.clone(),
                            Value::WidePointer {
                                data: Box::new(Value::Scalar(0x2200)),
                                metadata: Box::new(Value::Scalar(19)),
                            },
                        ],
                        &mut |name: &str, _: &[Value]| {
                            Err(InterpretError::UnknownCallee(name.into()))
                        },
                    )
                };
                match (run(&original), run(&f.graph)) {
                    (
                        Err(InterpretError::PoisonObserved(_)),
                        Err(InterpretError::PoisonObserved(_)),
                    ) => {}
                    (before, after) => assert_eq!(before, after),
                }
            }
        }
    }
}
