use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, SizeAlign, TyId,
    TypeShape,
};
use kfrc_son::interpret::{SemanticOutcome, Value};
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, InlineBudget, InlineSkip,
    InterpretError, Interpreter, NodeId, Opcode, ValueType, inline_calls, verify_graph,
};
use kfrc_body::fx_hash::HashMap;

struct Function {
    graph: FunctionGraph,
    int: TyId,
    control: NodeId,
    effect: NodeId,
    args: Vec<NodeId>,
}
impl Function {
    fn new(name: &str, args: usize, shift_types: bool) -> Self {
        let mut types = RecordedTypes::new();
        if shift_types {
            types.intern(TypeShape::Unit, None);
            types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        }
        let int = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            name,
            FunctionAbi {
                args: vec![PassMode::Direct; args],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(std::iter::repeat_n(ValueType::Rust(int), args));
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
        let args = (0..args)
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
            control,
            effect,
            args,
        }
    }
    fn node(&mut self, op: Opcode, args: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, args, payload, ValueType::Rust(ty))
    }
    fn ret(&mut self, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
    }
    fn call(&mut self, target: &FunctionGraph, args: &[NodeId], result: TyId) -> NodeId {
        let target = self.graph.add_callee(CallTarget {
            symbol: target.symbol.clone(),
            abi: target.abi.clone(),
        });
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(args);
        let call = self.graph.add_node(
            Opcode::Call,
            self.control,
            &inputs,
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(result),
            ]),
        );
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            1,
            ValueType::Effect,
        );
        self.node(Opcode::Projection, &[call], 2, result)
    }
    fn verify(&mut self) {
        verify_graph(&mut self.graph).unwrap();
    }
}
fn run(graph: &FunctionGraph, args: &[u64]) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(
        &args.iter().copied().map(Value::Scalar).collect::<Vec<_>>(),
        &mut |name: &str, _: &[Value]| Err(InterpretError::UnknownCallee(name.into())),
    )
}

#[test]
fn repeated_callee_import_preserves_types_added_by_an_intervening_callee() {
    let mut first = Function::new("first", 1, true);
    let nine = first.graph.add_constant(9, first.int);
    let sum = first.node(Opcode::Add, &[first.args[0], nine], 0, first.int);
    first.ret(sum);
    first.verify();

    let mut second = Function::new("second", 1, false);
    // Importing this source extends the caller's oracle even though its ABI
    // still uses u64. Reusing the first import must retain that extension.
    let extra_shape = TypeShape::Int { width: IntegerWidth::W32, signed: true };
    second.graph.oracle.intern(extra_shape.clone(), Some(SizeAlign { size: 4, align: 4 }));
    let eleven = second.graph.add_constant(11, second.int);
    let sum = second.node(Opcode::Add, &[second.args[0], eleven], 0, second.int);
    second.ret(sum);
    second.verify();

    let mut caller = Function::new("caller", 1, false);
    let a = caller.call(&first.graph, &[caller.args[0]], caller.int);
    let b = caller.call(&second.graph, &[a], caller.int);
    let c = caller.call(&first.graph, &[b], caller.int);
    caller.ret(c);
    caller.verify();
    let sources = HashMap::from_iter([
        (first.graph.symbol.clone(), first.graph),
        (second.graph.symbol.clone(), second.graph),
    ]);
    let stats = inline_calls(&mut caller.graph, &sources, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 3);
    assert!((0..caller.graph.oracle.table.len()).any(|index| {
        caller.graph.oracle.table.shape(TyId(index as u32)) == &extra_shape
    }));
    caller.verify();
    for input in [0, 1, 17, u64::MAX] {
        assert_eq!(run(&caller.graph, &[input]).unwrap(), SemanticOutcome::Return(Value::Scalar(input.wrapping_add(29))));
    }
}

#[test]
fn multiblock_callee_return_phi_remaps_independent_type_tables() {
    let mut callee = Function::new("choose", 1, true);
    let zero = callee.graph.add_constant(0, callee.int);
    let boolean = callee
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let condition = callee.node(
        Opcode::Compare,
        &[callee.args[0], zero],
        ComparePredicate::Equal as u32,
        boolean,
    );
    let branch = callee.graph.add_node(
        Opcode::If,
        callee.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    for (arm, answer) in [(0, 17), (1, 31)] {
        callee.control = callee.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            arm,
            ValueType::Control,
        );
        let value = callee.graph.add_constant(answer, callee.int);
        callee.ret(value);
    }
    callee.verify();
    let mut caller = Function::new("caller", 1, false);
    assert_ne!(caller.int, callee.int);
    let result = caller.call(&callee.graph, &caller.args.clone(), caller.int);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let stats = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 1);
    assert!(stats.skipped.is_empty());
    verify_graph(&mut caller.graph).unwrap();
    assert!(
        !caller
            .graph
            .ids()
            .any(|n| caller.graph.op(n) == Opcode::Call)
    );
    assert!(
        caller
            .graph
            .ids()
            .any(|n| caller.graph.op(n) == Opcode::Phi)
    );
    for input in [0, 1, 99, u64::MAX] {
        assert_eq!(
            run(&caller.graph, &[input]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(if input == 0 { 17 } else { 31 }))
        );
    }
}

#[test]
fn unused_poison_argument_is_still_observed_after_inlining() {
    let mut callee = Function::new("ignore", 1, true);
    let answer = callee.graph.add_constant(42, callee.int);
    callee.ret(answer);
    callee.verify();
    let mut caller = Function::new("caller", 0, false);
    let poison = caller.node(Opcode::Undefined, &[], 0, caller.int);
    let result = caller.call(&callee.graph, &[poison], caller.int);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let stats = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 1);
    assert!(matches!(
        run(&caller.graph, &[]),
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn partially_poison_return_is_observed_before_the_caller_projects_a_good_field() {
    let mut callee = Function::new("bad_pair", 0, true);
    let pair = callee
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![callee.int, callee.int]), None);
    callee.graph.abi.ret = PassMode::Pair;
    let answer = callee.graph.add_constant(42, callee.int);
    let poison = callee.node(Opcode::Undefined, &[], 0, callee.int);
    let value = callee.node(Opcode::Aggregate, &[answer, poison], 0, pair);
    callee.ret(value);
    callee.verify();
    let mut caller = Function::new("caller", 0, false);
    let pair = caller
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![caller.int, caller.int]), None);
    let value = caller.call(&callee.graph, &[], pair);
    let result = caller.node(Opcode::Field, &[value], 0, caller.int);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    assert_eq!(
        inline_calls(&mut caller.graph, &provider, InlineBudget::default())
            .unwrap()
            .calls_inlined,
        1
    );
    assert!(matches!(
        run(&caller.graph, &[]),
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn recursive_scc_and_budget_leave_calls_unchanged() {
    let mut recursive = Function::new("recursive", 1, true);
    let signature = recursive.graph.clone();
    let result = recursive.call(&signature, &recursive.args.clone(), recursive.int);
    recursive.ret(result);
    recursive.verify();
    let mut caller = Function::new("caller", 1, false);
    let result = caller.call(&recursive.graph, &caller.args.clone(), caller.int);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(recursive.graph.symbol.clone(), recursive.graph)]);
    let before = caller.graph.dump();
    let stats = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 0);
    assert_eq!(stats.skipped[0].1, InlineSkip::Recursive);
    assert_eq!(caller.graph.dump(), before);
    let stats = inline_calls(
        &mut caller.graph,
        &provider,
        InlineBudget {
            max_calls: 0,
            free_nodes: 0,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert_eq!(stats.skipped[0].1, InlineSkip::Budget);
    assert_eq!(caller.graph.dump(), before);
}

#[test]
fn conflicting_nominal_field_types_skip_atomically() {
    let mut callee = Function::new("record", 0, true);
    let adt = callee.graph.oracle.intern(
        TypeShape::Adt {
            name: "Record".into(),
        },
        None,
    );
    callee.graph.oracle.record_semantic_aggregate(
        adt,
        SemanticAggregate {
            fields: vec![callee.int],
            variants: vec![],
        },
    );
    let value = callee.graph.add_constant(1, callee.int);
    let record = callee.node(Opcode::Aggregate, &[value], 0, adt);
    callee.ret(record);
    callee.verify();
    let mut caller = Function::new("caller", 0, false);
    let boolean = caller
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let adt = caller.graph.oracle.intern(
        TypeShape::Adt {
            name: "Record".into(),
        },
        None,
    );
    caller.graph.oracle.record_semantic_aggregate(
        adt,
        SemanticAggregate {
            fields: vec![boolean],
            variants: vec![],
        },
    );
    let record = caller.call(&callee.graph, &[], adt);
    let result = caller.node(Opcode::Field, &[record], 0, boolean);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let before = caller.graph.dump();
    let stats = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 0);
    assert_eq!(stats.skipped[0].1, InlineSkip::TypeConflict);
    assert_eq!(caller.graph.dump(), before);
    assert_eq!(
        caller.graph.oracle.semantic().field_type(adt, None, 0),
        Some(boolean)
    );
}

/// Inlines `address() -> &private_static` for a local object with `flags`.
fn inline_local_object(
    unnamed_address: bool,
    writable: bool,
    mutable_child: bool,
) -> (usize, Vec<InlineSkip>, usize) {
    let mut callee = Function::new("address", 0, true);
    let pointer = callee.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: kfrc_body::Mutability::Not,
            pointee: callee.int,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    callee.graph.symbols.push(("private_static".into(), 0));
    callee.graph.data_objects.push(kfrc_body::DataObject {
        symbol: "private_static".into(),
        bytes: vec![0; 8],
        align: 8,
        unnamed_address,
        writable,
        ..Default::default()
    });
    if mutable_child {
        callee.graph.data_objects[0]
            .relocations
            .push(kfrc_body::DataRelocation {
                offset: 0,
                target: "mutable_child".into(),
                addend: 0,
            });
        callee.graph.data_objects.push(kfrc_body::DataObject {
            symbol: "mutable_child".into(),
            bytes: vec![0; 8],
            align: 8,
            writable: true,
            ..Default::default()
        });
    }
    let address = callee.node(Opcode::SymbolAddress, &[], 0, pointer);
    callee.ret(address);
    callee.verify();
    let mut caller = Function::new("caller", 0, false);
    let pointer = caller.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: kfrc_body::Mutability::Not,
            pointee: caller.int,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let result = caller.call(&callee.graph, &[], pointer);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let budget = InlineBudget {
        copy_anonymous_constants: true,
        ..InlineBudget::default()
    };
    let stats = inline_calls(&mut caller.graph, &provider, budget).unwrap();
    let reasons = stats.skipped.iter().map(|(_, reason)| *reason).collect();
    (
        stats.calls_inlined,
        reasons,
        caller.graph.data_objects.len(),
    )
}

#[test]
fn read_only_anonymous_constants_copy_into_the_caller() {
    assert_eq!(inline_local_object(true, false, false), (1, vec![], 1));
    // Writable memory keeps its single identity even when anonymous.
    assert_eq!(
        inline_local_object(true, true, false),
        (0, vec![InlineSkip::DataConflict], 0)
    );
    // A copyable constant may still point at state with a unique identity.
    assert_eq!(
        inline_local_object(true, false, true),
        (0, vec![InlineSkip::DataConflict], 0)
    );
}

#[test]
fn local_data_identity_is_not_duplicated_into_another_cgu() {
    let mut callee = Function::new("address", 0, true);
    let pointer = callee.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: kfrc_body::Mutability::Not,
            pointee: callee.int,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    callee.graph.symbols.push(("private_static".into(), 0));
    callee.graph.data_objects.push(kfrc_body::DataObject {
        symbol: "private_static".into(),
        bytes: vec![0; 8],
        align: 8,
        ..Default::default()
    });
    let address = callee.node(Opcode::SymbolAddress, &[], 0, pointer);
    callee.ret(address);
    callee.verify();
    let mut caller = Function::new("caller", 0, false);
    let pointer = caller.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: kfrc_body::Mutability::Not,
            pointee: caller.int,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let result = caller.call(&callee.graph, &[], pointer);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let before = caller.graph.dump();
    let stats = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 0);
    assert_eq!(stats.skipped[0].1, InlineSkip::DataConflict);
    assert_eq!(caller.graph.dump(), before);
    assert!(caller.graph.data_objects.is_empty());
}

#[test]
fn dead_local_data_does_not_prevent_inlining_a_pure_wrapper() {
    let mut callee = Function::new("increment", 1, true);
    callee.graph.data_objects.push(kfrc_body::DataObject {
        symbol: "dead_private_state".into(),
        bytes: vec![0; 8],
        align: 8,
        writable: true,
        ..Default::default()
    });
    let one = callee.graph.add_constant(1, callee.int);
    let value = callee.node(Opcode::Add, &[callee.args[0], one], 0, callee.int);
    callee.ret(value);
    callee.verify();
    let mut caller = Function::new("caller", 1, false);
    let arg = caller.args[0];
    let result = caller.call(&callee.graph, &[arg], caller.int);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let stats = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 1, "{:?}", stats.skipped);
    assert!(caller.graph.data_objects.is_empty());
    for n in [0u64, 1, u64::MAX] {
        assert_eq!(
            run(&caller.graph, &[n]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(n.wrapping_add(1)))
        );
    }
}

#[test]
fn inline_seeds_expose_aggregate_fields_without_resweeping_unrelated_values() {
    let mut callee = Function::new("first", 1, true);
    let pair = callee
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![callee.int, callee.int]), None);
    let start = callee.graph.start;
    callee.graph.set_value_type(
        start,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(pair),
        ]),
    );
    callee
        .graph
        .set_value_type(callee.args[0], ValueType::Rust(pair));
    callee.graph.abi.args[0] = PassMode::Pair;
    let first = callee.node(Opcode::Field, &[callee.args[0]], 0, callee.int);
    let one = callee.graph.add_constant(1, callee.int);
    let result = callee.node(Opcode::Add, &[first, one], 0, callee.int);
    callee.ret(result);
    callee.verify();
    let mut caller = Function::new("caller", 1, false);
    let pair = caller
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![caller.int, caller.int]), None);
    let forty = caller.graph.add_constant(40, caller.int);
    let two = caller.graph.add_constant(2, caller.int);
    let aggregate = caller.node(Opcode::Aggregate, &[forty, two], 0, pair);
    let result = caller.call(&callee.graph, &[aggregate], caller.int);
    // Unchanged arithmetic belongs to the caller and is unrelated to the
    // splice. It is deliberately long enough to catch full rule rescanning.
    let mut unrelated = caller.args[0];
    for _ in 0..120 {
        unrelated = caller.node(Opcode::Mul, &[unrelated, two], 0, caller.int);
    }
    let result = caller.node(Opcode::Add, &[result, unrelated], 0, caller.int);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let inline = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(inline.calls_inlined, 1);
    assert!(!inline.dirty_nodes.is_empty());
    let optimized = kfrc_son::optimize_specialized_seams(
        &mut caller.graph,
        &inline.dirty_nodes,
        &inline.argument_slots,
    )
    .unwrap();
    assert_eq!(optimized.fields_folded, 1);
    assert!(optimized.constants_folded >= 1);
    assert!(optimized.rule_nodes_visited < 50, "{optimized:?}");
    assert!(
        optimized.rule_nodes_indexed >= 120,
        "index cost must remain visible"
    );
    assert_eq!(
        run(&caller.graph, &[7]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(41))
    );
}

#[test]
fn inline_repeat_return_imports_length_and_exposes_projection_seam() {
    let mut callee = Function::new("repeat", 1, true);
    let array = callee.graph.oracle.intern(
        TypeShape::Array {
            element: callee.int,
            length: 4096,
        },
        None,
    );
    let repeated = callee.node(Opcode::Repeat, &[callee.args[0]], 0, array);
    callee.ret(repeated);
    callee.verify();
    let mut caller = Function::new("caller", 1, false);
    let array = caller.graph.oracle.intern(
        TypeShape::Array {
            element: caller.int,
            length: 4096,
        },
        None,
    );
    let repeated = caller.call(&callee.graph, &[caller.args[0]], array);
    let result = caller.node(Opcode::Field, &[repeated], 4095, caller.int);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let inline = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(inline.calls_inlined, 1);
    assert!(caller.graph.len() < 40);
    let optimized = kfrc_son::optimize_specialized_seams(
        &mut caller.graph,
        &inline.dirty_nodes,
        &inline.argument_slots,
    )
    .unwrap();
    assert_eq!(optimized.fields_folded, 1);
    assert_eq!(
        run(&caller.graph, &[42]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
}

#[test]
fn variant_switch_and_witness_keep_proof_edges_when_inlined() {
    fn enumeration(f: &mut Function) -> TyId {
        let ty = f.graph.oracle.intern(
            TypeShape::Adt {
                name: "InlineOptional".into(),
            },
            None,
        );
        f.graph.oracle.record_semantic_aggregate(
            ty,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![], vec![f.int]],
            },
        );
        f.graph.oracle.record_semantic_enum(ty, vec![0, 1]);
        f.graph
            .oracle
            .record_semantic_inhabited_variants(ty, vec![0, 1]);
        f.graph.set_value_type(
            f.graph.start,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        f.graph.set_value_type(f.args[0], ValueType::Rust(ty));
        ty
    }
    let mut callee = Function::new("unwrap_or_zero", 1, true);
    enumeration(&mut callee);
    callee.graph.variant_switches.push(vec![1]);
    let branch = callee.graph.add_node(
        Opcode::VariantSwitch,
        callee.control,
        &[callee.args[0]],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    for arm in 0..2 {
        callee.control = callee.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            arm,
            ValueType::Control,
        );
        let witness = callee.graph.add_node(
            Opcode::VariantWitness,
            callee.control,
            &[callee.args[0], callee.control],
            1 - arm,
            ValueType::Witness,
        );
        let value = if arm == 0 {
            let payload = callee.graph.intern_variant_field(1, 0);
            callee.node(
                Opcode::VariantField,
                &[callee.args[0], witness],
                payload,
                callee.int,
            )
        } else {
            callee.graph.add_constant(0, callee.int)
        };
        callee.ret(value);
    }
    callee.verify();
    let mut caller = Function::new("caller", 1, false);
    let ty = enumeration(&mut caller);
    // Existing side tables deliberately differ at index zero.
    caller.graph.variant_switches.push(vec![0]);
    let result = caller.call(&callee.graph, &[caller.args[0]], caller.int);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let stats = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 1, "{stats:?}");
    assert_eq!(
        caller.graph.oracle.semantic().inhabited_variants(ty),
        Some(&[0, 1][..])
    );
    for (variant, fields, expected) in [(0, vec![], 0), (1, vec![Value::Scalar(37)], 37)] {
        let actual = Interpreter::new(&caller.graph)
            .run_values(
                &[Value::Variant { variant, fields }],
                &mut |_: &str, _: &[Value]| unreachable!(),
            )
            .unwrap();
        assert_eq!(actual, SemanticOutcome::Return(Value::Scalar(expected)));
    }
    for node in caller
        .graph
        .ids()
        .filter(|&n| caller.graph.op(n) == Opcode::VariantWitness)
    {
        assert_eq!(caller.graph.control(node), caller.graph.input(node, 1));
    }
}

#[test]
fn scalar_inline_does_not_require_an_unused_local_identity() {
    let mut callee = Function::new("identity", 1, true);
    callee.ret(callee.args[0]);
    callee.verify();
    let mut caller = Function::new("caller", 1, false);
    let pointer = caller.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: kfrc_body::Mutability::Mut,
            pointee: caller.int,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    caller.node(Opcode::LocalSlot, &[], u32::MAX, pointer);
    let result = caller.call(&callee.graph, &[caller.args[0]], caller.int);
    caller.ret(result);
    caller.verify();
    let provider = HashMap::from_iter([(callee.graph.symbol.clone(), callee.graph)]);
    let stats = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 1, "{stats:?}");
    assert_eq!(
        run(&caller.graph, &[42]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
}

#[test]
fn later_invalid_source_rolls_back_earlier_successful_splice() {
    let mut valid = Function::new("identity", 1, true);
    valid.ret(valid.args[0]);
    valid.verify();
    let mut invalid = Function::new("invalid", 1, true);
    let malformed = invalid.node(Opcode::Add, &[invalid.args[0]], 0, invalid.int);
    invalid.ret(malformed);
    let mut caller = Function::new("caller", 1, false);
    let first = caller.call(&valid.graph, &[caller.args[0]], caller.int);
    let second = caller.call(&invalid.graph, &[first], caller.int);
    caller.ret(second);
    caller.verify();
    let provider = HashMap::from_iter([
        (valid.graph.symbol.clone(), valid.graph),
        (invalid.graph.symbol.clone(), invalid.graph),
    ]);
    let before = caller.graph.dump();
    assert!(inline_calls(&mut caller.graph, &provider, InlineBudget::default()).is_err());
    assert_eq!(caller.graph.dump(), before);
}

#[test]
fn inlining_retains_fixed_assumption_even_when_its_value_is_unused() {
    let mut callee = Function::new("assumption_then_identity", 1, false);
    let boolean = callee
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let zero = callee.graph.add_constant(0, callee.int);
    let condition = callee.node(
        Opcode::Compare,
        &[callee.args[0], zero],
        ComparePredicate::NotEqual as u32,
        boolean,
    );
    let assume = callee.graph.add_node(
        Opcode::Assume,
        callee.control,
        &[callee.effect, condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    callee.control = callee.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[assume],
        0,
        ValueType::Control,
    );
    callee.effect = callee.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[assume],
        1,
        ValueType::Effect,
    );
    callee.ret(callee.args[0]);
    callee.verify();
    let mut caller = Function::new("caller", 1, true);
    caller.call(&callee.graph, &caller.args.clone(), caller.int);
    let value = caller.graph.add_constant(42, caller.int);
    caller.ret(value);
    caller.verify();
    let functions = HashMap::from_iter([("assumption_then_identity".into(), callee.graph)]);
    assert_eq!(
        kfrc_son::fold_identity_calls(&mut caller.graph, &functions, None)
            .unwrap()
            .calls_folded,
        0
    );
    assert_eq!(
        inline_calls(&mut caller.graph, &functions, InlineBudget::default())
            .unwrap()
            .calls_inlined,
        1
    );
    assert_eq!(
        caller
            .graph
            .ids()
            .filter(|&n| caller.graph.op(n) == Opcode::Assume)
            .count(),
        1
    );
    assert!(matches!(
        run(&caller.graph, &[0]),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert_eq!(
        run(&caller.graph, &[1]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
}

/// `x + 1 + 1 + ...` with `adds` additions.
fn chain(name: &str, adds: usize) -> Function {
    let mut callee = Function::new(name, 1, false);
    let mut value = callee.args[0];
    for _ in 0..adds {
        let one = callee.graph.add_constant(1, callee.int);
        value = callee.node(Opcode::Add, &[value, one], 0, callee.int);
    }
    callee.ret(value);
    callee.verify();
    callee
}

/// Calls `big` then `small` on the argument and sums both results.
fn two_calls(big: &Function, small: &Function) -> Function {
    let mut caller = Function::new("caller", 1, false);
    let argument = caller.args[0];
    let first = caller.call(&big.graph, &[argument], caller.int);
    let second = caller.call(&small.graph, &[argument], caller.int);
    let sum = caller.node(Opcode::Add, &[first, second], 0, caller.int);
    caller.ret(sum);
    caller.verify();
    caller
}

fn remaining_calls(graph: &FunctionGraph) -> Vec<String> {
    graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::Call)
        .map(|node| graph.callees[graph.payload(node) as usize].symbol.clone())
        .collect()
}

#[test]
fn smallest_callee_is_inlined_first_and_large_callees_stay_calls() {
    let big = chain("big", 60);
    let small = chain("small", 3);
    let provider = HashMap::from_iter([
        (big.graph.symbol.clone(), big.graph.clone()),
        (small.graph.symbol.clone(), small.graph.clone()),
    ]);
    // One costed inline: the later, smaller call site wins it.
    let mut caller = two_calls(&big, &small);
    let budget = InlineBudget {
        max_calls: 1,
        free_nodes: 0,
        ..InlineBudget::default()
    };
    let stats = inline_calls(&mut caller.graph, &provider, budget).unwrap();
    assert_eq!(stats.calls_inlined, 1);
    assert_eq!(remaining_calls(&caller.graph), ["big"]);

    // Above the callee size limit the call stays; below it inlines.
    let mut caller = two_calls(&big, &small);
    let budget = InlineBudget {
        max_callee_nodes: big.graph.ids().count() - 1,
        ..InlineBudget::default()
    };
    let stats = inline_calls(&mut caller.graph, &provider, budget).unwrap();
    assert_eq!(remaining_calls(&caller.graph), ["big"]);
    assert!(stats.skipped.contains(&("big".into(), InlineSkip::Size)));
}

struct SoleCaller(HashMap<String, FunctionGraph>);
impl kfrc_son::GraphProvider for SoleCaller {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph> {
        self.0.get(symbol)
    }
    fn sole_call_site(&self, symbol: &str) -> bool {
        symbol == "big"
    }
}

#[test]
fn a_sole_call_site_inlines_regardless_of_callee_size() {
    let big = chain("big", 60);
    let small = chain("small", 3);
    let provider = SoleCaller(HashMap::from_iter([
        (big.graph.symbol.clone(), big.graph.clone()),
        (small.graph.symbol.clone(), small.graph.clone()),
    ]));
    let mut caller = two_calls(&big, &small);
    let budget = InlineBudget {
        max_callee_nodes: 1,
        ..InlineBudget::default()
    };
    inline_calls(&mut caller.graph, &provider, budget).unwrap();
    // `small` exceeds the limit and has other callers; `big` is local and sole.
    assert_eq!(remaining_calls(&caller.graph), ["small"]);
}

#[test]
fn observe_costs_preserves_legacy_choices_and_semantics() {
    use kfrc_son::analysis::cost::GenericCostModel;
    use kfrc_son::{InlineCostPolicy, inline_calls_with_cost};
    let big = chain("big", 60);
    let small = chain("small", 3);
    let provider = HashMap::from_iter([
        ("big".into(), big.graph.clone()),
        ("small".into(), small.graph.clone()),
    ]);
    let mut legacy = two_calls(&big, &small);
    let mut observed = legacy.graph.clone();
    let budget = InlineBudget {
        max_calls: 1,
        free_nodes: 0,
        ..InlineBudget::default()
    };
    let old = inline_calls(&mut legacy.graph, &provider, budget).unwrap();
    let new = inline_calls_with_cost(
        &mut observed,
        &provider,
        budget,
        InlineCostPolicy::Observe,
        &GenericCostModel,
    )
    .unwrap();
    assert_eq!(observed.dump(), legacy.graph.dump());
    assert_eq!(old.skipped, new.skipped);
    assert_eq!(old.calls_inlined, new.calls_inlined);
    assert_eq!(new.decisions.len(), 2);
    assert!(new.decisions.iter().all(|d| d.estimate.is_some()));
    assert!(new.cost_nodes_visited <= budget.max_nodes);
}

#[test]
fn size_cost_policy_is_distinct_from_node_limits_but_keeps_growth_budget() {
    use kfrc_son::analysis::cost::GenericCostModel;
    use kfrc_son::{InlineCostPolicy, inline_calls_with_cost};
    let callee = chain("big", 4);
    let provider = HashMap::from_iter([("big".into(), callee.graph.clone())]);
    let mut caller = Function::new("caller", 1, false);
    let value = caller.call(&callee.graph, &[caller.args[0]], caller.int);
    caller.ret(value);
    caller.verify();
    let original = caller.graph.clone();
    let budget = InlineBudget {
        max_callee_nodes: 0,
        ..InlineBudget::default()
    };
    let policy = InlineCostPolicy::CodeSize {
        max_units: 12,
        free_units: 0,
    };
    let stats = inline_calls_with_cost(
        &mut caller.graph,
        &provider,
        budget,
        policy,
        &GenericCostModel,
    )
    .unwrap();
    assert_eq!(stats.calls_inlined, 1);
    for x in [0, 5, u64::MAX] {
        assert_eq!(
            run(&caller.graph, &[x]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(x.wrapping_add(4)))
        );
    }
    let mut rejected = original.clone();
    let stats = inline_calls_with_cost(
        &mut rejected,
        &provider,
        budget,
        InlineCostPolicy::CodeSize {
            max_units: 0,
            free_units: 0,
        },
        &GenericCostModel,
    )
    .unwrap();
    assert_eq!(stats.skipped, [("big".into(), InlineSkip::Cost)]);
    assert_eq!(stats.decisions[0].rejection, Some(InlineSkip::Cost));
    assert_eq!(rejected.dump(), original.dump());
    let mut rejected = original.clone();
    let stats = inline_calls_with_cost(
        &mut rejected,
        &provider,
        InlineBudget {
            max_nodes: 1,
            ..budget
        },
        policy,
        &GenericCostModel,
    )
    .unwrap();
    assert_eq!(stats.skipped, [("big".into(), InlineSkip::Budget)]);
    assert_eq!(stats.cost_nodes_visited, 0);
    assert_eq!(rejected.dump(), original.dump());
}

#[test]
fn unknown_cost_is_not_free_and_summaries_are_cached_for_repeated_calls() {
    use kfrc_son::analysis::cost::{CostKind, CostModel, InstructionCost};
    use kfrc_son::{InlineCostPolicy, inline_calls_with_cost};
    struct Unknown;
    impl CostModel for Unknown {
        fn name(&self) -> &'static str {
            "test-unknown"
        }
        fn instruction_cost(&self, _: &FunctionGraph, _: NodeId, _: CostKind) -> InstructionCost {
            InstructionCost::Unknown
        }
    }
    let callee = chain("same", 2);
    let provider = HashMap::from_iter([("same".into(), callee.graph.clone())]);
    let mut caller = two_calls(&callee, &callee);
    let original = caller.graph.dump();
    let stats = inline_calls_with_cost(
        &mut caller.graph,
        &provider,
        InlineBudget::default(),
        InlineCostPolicy::CodeSize {
            max_units: u64::MAX,
            free_units: u64::MAX,
        },
        &Unknown,
    )
    .unwrap();
    assert_eq!(stats.calls_inlined, 0);
    assert_eq!(stats.skipped.len(), 2);
    assert!(
        stats
            .skipped
            .iter()
            .all(|(_, reason)| *reason == InlineSkip::UnknownCost)
    );
    assert!(stats.decisions.iter().all(|decision| !decision.free));
    assert_eq!(stats.cost_nodes_visited, callee.graph.live_count());
    assert_eq!(caller.graph.dump(), original);
}

#[test]
fn ir_costs_separate_static_work_structure_and_unknown_callee_time() {
    use kfrc_son::analysis::cost::{
        CostKind, CostModel, GenericCostModel, InstructionCost, estimate_graph,
    };
    let mut f = Function::new("costs", 2, false);
    let add = f.node(Opcode::Add, &f.args.clone(), 0, f.int);
    let mul = f.node(Opcode::Mul, &f.args.clone(), 0, f.int);
    let model = GenericCostModel;
    assert_eq!(
        model.instruction_cost(&f.graph, add, CostKind::CodeSize),
        model.instruction_cost(&f.graph, mul, CostKind::CodeSize)
    );
    let InstructionCost::Known(add_work) =
        model.instruction_cost(&f.graph, add, CostKind::ReciprocalThroughput)
    else {
        panic!()
    };
    let InstructionCost::Known(mul_work) =
        model.instruction_cost(&f.graph, mul, CostKind::ReciprocalThroughput)
    else {
        panic!()
    };
    assert!(mul_work > add_work);
    f.ret(mul);
    f.verify();
    let before = estimate_graph(&f.graph, &model);
    // Structural proof/control bookkeeping does not have the same cost as an
    // arithmetic operation. It still contributes to compiler work.
    f.graph
        .add_node(Opcode::Region, NodeId::NONE, &[], 0, ValueType::Control);
    let after = estimate_graph(&f.graph, &model);
    assert_eq!(after.nodes, before.nodes + 1);
    assert_eq!(after.code_size, before.code_size);
    assert_eq!(after.static_work, before.static_work);
    let callee = chain("callee", 2);
    let mut caller = Function::new("caller", 1, false);
    let value = caller.call(&callee.graph, &[caller.args[0]], caller.int);
    caller.ret(value);
    caller.verify();
    let call = caller
        .graph
        .ids()
        .find(|&n| caller.graph.op(n) == Opcode::Call)
        .unwrap();
    assert!(matches!(
        model.instruction_cost(&caller.graph, call, CostKind::CodeSize),
        InstructionCost::Known(_)
    ));
    assert_eq!(
        model.instruction_cost(&caller.graph, call, CostKind::Latency),
        InstructionCost::Unknown
    );
    assert!(
        estimate_graph(&caller.graph, &model)
            .static_work
            .total()
            .is_none()
    );
}

#[test]
fn cost_arithmetic_overflow_cannot_make_a_graph_look_cheaper() {
    use kfrc_son::analysis::cost::{CostKind, CostModel, InstructionCost, estimate_graph};
    struct Huge;
    impl CostModel for Huge {
        fn name(&self) -> &'static str {
            "test-overflow"
        }
        fn instruction_cost(&self, _: &FunctionGraph, _: NodeId, _: CostKind) -> InstructionCost {
            InstructionCost::Known(u64::MAX)
        }
    }
    let f = chain("overflow", 2);
    let estimate = estimate_graph(&f.graph, &Huge);
    assert!(estimate.code_size.total().is_none());
    assert!(estimate.static_work.total().is_none());
}

#[test]
fn modeled_size_orders_calls_by_cost_instead_of_node_count() {
    use kfrc_son::analysis::cost::GenericCostModel;
    use kfrc_son::{InlineCostPolicy, inline_calls_with_cost};
    let mut shifts = Function::new("shifts", 1, false);
    let mut x = shifts.args[0];
    for _ in 0..2 {
        x = shifts.node(Opcode::ShiftLeft, &[x, shifts.args[0]], 0, shifts.int);
    }
    shifts.ret(x);
    shifts.verify();
    let mut adds = Function::new("adds", 1, false);
    let mut x = adds.args[0];
    for _ in 0..3 {
        x = adds.node(Opcode::Add, &[x, adds.args[0]], 0, adds.int);
    }
    adds.ret(x);
    adds.verify();
    assert!(shifts.graph.live_count() < adds.graph.live_count());
    let provider = HashMap::from_iter([
        ("shifts".into(), shifts.graph.clone()),
        ("adds".into(), adds.graph.clone()),
    ]);
    let mut caller = two_calls(&shifts, &adds);
    let stats = inline_calls_with_cost(
        &mut caller.graph,
        &provider,
        InlineBudget {
            max_calls: 1,
            free_nodes: 0,
            ..InlineBudget::default()
        },
        InlineCostPolicy::CodeSize {
            max_units: 64,
            free_units: 0,
        },
        &GenericCostModel,
    )
    .unwrap();
    assert_eq!(stats.calls_inlined, 1);
    assert_eq!(remaining_calls(&caller.graph), ["shifts"]);
    assert_eq!(stats.decisions[0].symbol, "adds");
    assert_eq!(stats.decisions[1].rejection, Some(InlineSkip::Budget));
    verify_graph(&mut caller.graph).unwrap();
}
