use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, SizeAlign, TyId,
    TypeShape,
};
use kfrc_son::interpret::{SemanticOutcome, Value};
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, InlineBudget, InlineSkip,
    InterpretError, Interpreter, NodeId, Opcode, ValueType, inline_calls, verify_graph,
};
use std::collections::HashMap;

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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
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
    let provider = HashMap::from([(recursive.graph.symbol.clone(), recursive.graph)]);
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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
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
fn inline_local_object(unnamed_address: bool, writable: bool) -> (usize, Vec<InlineSkip>, usize) {
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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
    let budget = InlineBudget {
        copy_anonymous_constants: true,
        ..InlineBudget::default()
    };
    let stats = inline_calls(&mut caller.graph, &provider, budget).unwrap();
    let reasons = stats.skipped.iter().map(|(_, reason)| *reason).collect();
    (stats.calls_inlined, reasons, caller.graph.data_objects.len())
}

#[test]
fn read_only_anonymous_constants_copy_into_the_caller() {
    assert_eq!(inline_local_object(true, false), (1, vec![], 1));
    // Writable memory keeps its single identity even when anonymous.
    assert_eq!(
        inline_local_object(true, true),
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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
    let before = caller.graph.dump();
    let stats = inline_calls(&mut caller.graph, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 0);
    assert_eq!(stats.skipped[0].1, InlineSkip::DataConflict);
    assert_eq!(caller.graph.dump(), before);
    assert!(caller.graph.data_objects.is_empty());
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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
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
    let provider = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
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
    let provider = HashMap::from([
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
    let functions = HashMap::from([("assumption_then_identity".into(), callee.graph)]);
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
    let provider = HashMap::from([
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
    let provider = SoleCaller(HashMap::from([
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
