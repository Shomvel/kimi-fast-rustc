//! Metadata identity never replaces a fixed whole-value representation check.
use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes,
    ScalarValidRange, ScalarValidity, SizeAlign, TyId, TypeShape,
};
use kfrc_son::{
    CallTarget, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode,
    SemanticMemory, SemanticOutcome, Value, ValueType, optimize_graph, verify_graph,
};

struct Types {
    oracle: RecordedTypes,
    word: TyId,
    thin: TyId,
    slice: TyId,
    string: TyId,
    source: TyId,
    target: TyId,
}

fn wide(types: &mut RecordedTypes, pointee: TyId, thin: TyId, metadata: TyId) -> TyId {
    let ty = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        ty,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: thin,
                    offset: 0,
                },
                FieldLayout {
                    ty: metadata,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    ty
}

impl Types {
    fn new() -> Self {
        let mut oracle = RecordedTypes::new();
        let word = oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let byte = oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let unit = oracle.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
        let thin = oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: unit,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let slice = oracle.intern(TypeShape::Slice { element: byte }, None);
        let string = oracle.intern(TypeShape::Str, None);
        let source = wide(&mut oracle, slice, thin, word);
        let target = wide(&mut oracle, string, thin, word);
        Self {
            oracle,
            word,
            thin,
            slice,
            string,
            source,
            target,
        }
    }

    fn reference(&mut self, pointee: TyId) -> TyId {
        let ty = self.oracle.intern(
            TypeShape::Reference {
                mutability: Mutability::Not,
                pointee,
            },
            Some(SizeAlign { size: 16, align: 8 }),
        );
        let fields = self.oracle.aggregates[&self.source].clone();
        self.oracle.record_aggregate(ty, fields);
        self.oracle.record_scalar_validity(
            ty,
            ScalarValidity::Pair {
                first: Some(ScalarValidRange::new(64, 1, u128::from(u64::MAX)).unwrap()),
                second: None,
                second_offset: 8,
            },
        );
        ty
    }
}

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    thin: TyId,
    source: TyId,
    target: TyId,
    control: NodeId,
    effect: NodeId,
    args: Vec<NodeId>,
}

impl Fixture {
    fn new(types: Types, args: &[TyId]) -> Self {
        let mut graph = FunctionGraph::new(
            "representation_metadata",
            FunctionAbi {
                args: vec![PassMode::Direct; args.len()],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types.oracle,
        );
        graph.level = GraphLevel::Specialized;
        let mut results = vec![ValueType::Control, ValueType::Effect];
        results.extend(args.iter().copied().map(ValueType::Rust));
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(results),
        );
        graph.start = start;
        let control = project(&mut graph, start, 0, ValueType::Control);
        let effect = project(&mut graph, start, 1, ValueType::Effect);
        let args = args
            .iter()
            .enumerate()
            .map(|(index, ty)| project(&mut graph, start, index as u32 + 2, ValueType::Rust(*ty)))
            .collect();
        Self {
            graph,
            word: types.word,
            thin: types.thin,
            source: types.source,
            target: types.target,
            control,
            effect,
            args,
        }
    }

    fn node(&mut self, op: Opcode, inputs: &[NodeId], index: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, inputs, index, ValueType::Rust(ty))
    }

    fn fixed(
        &mut self,
        op: Opcode,
        operands: &[NodeId],
        payload: u32,
        ty: Option<TyId>,
    ) -> Option<NodeId> {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(operands);
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(ty.map(ValueType::Rust));
        let event = self.graph.add_node(
            op,
            self.control,
            &inputs,
            payload,
            ValueType::Tuple(outputs),
        );
        self.control = project(&mut self.graph, event, 0, ValueType::Control);
        self.effect = project(&mut self.graph, event, 1, ValueType::Effect);
        ty.map(|ty| project(&mut self.graph, event, 2, ValueType::Rust(ty)))
    }

    fn transmute(&mut self, source: NodeId, target: TyId) -> NodeId {
        self.fixed(Opcode::TransmuteValue, &[source], 0, Some(target))
            .unwrap()
    }

    fn metadata(&mut self, source: NodeId) -> NodeId {
        let ty = self
            .graph
            .oracle
            .semantic()
            .field_type(self.graph.rust_type(source).unwrap(), None, 1)
            .unwrap();
        self.node(Opcode::Field, &[source], 1, ty)
    }

    fn pointer(&mut self, data: NodeId, metadata: NodeId, ty: TyId) -> NodeId {
        self.node(Opcode::WidePtr, &[data, metadata], 0, ty)
    }

    fn call(&mut self, symbol: &str) {
        let callee = self.graph.add_callee(CallTarget {
            symbol: symbol.into(),
            abi: FunctionAbi {
                args: vec![],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        self.fixed(Opcode::Call, &[], callee, Some(self.word));
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

    fn optimize(&mut self) {
        verify_graph(&mut self.graph).unwrap();
        optimize_graph(&mut self.graph);
        verify_graph(&mut self.graph).unwrap();
    }
}

fn project(graph: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

fn value(data: u64, len: u64) -> Value {
    Value::WidePointer {
        data: Box::new(Value::Scalar(data)),
        metadata: Box::new(Value::Scalar(len)),
    }
}

fn run(graph: &FunctionGraph, args: &[Value]) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(args, &mut |symbol: &str, _: &[Value]| {
        Err(InterpretError::UnknownCallee(symbol.into()))
    })
}

fn count(graph: &FunctionGraph, opcode: Opcode) -> usize {
    graph.ids().filter(|&node| graph.op(node) == opcode).count()
}

fn converted_metadata(graph: &FunctionGraph) -> usize {
    graph
        .ids()
        .filter(|&node| {
            if graph.op(node) != Opcode::Field || graph.payload(node) != 1 {
                return false;
            }
            let source = graph.resolve(graph.input(node, 0));
            graph.op(source) == Opcode::Projection
                && graph.payload(source) == 2
                && graph.op(graph.resolve(graph.input(source, 0))) == Opcode::TransmuteValue
        })
        .count()
}

fn enabled() -> bool {
    std::env::var_os("KFRC_REPR_WIDEPTR_METADATA").is_some_and(|value| value == "1")
}

#[test]
fn two_representation_barriers_reuse_length_and_keep_whole_fixed_observations() {
    let mut types = Types::new();
    let middle = types.reference(types.string);
    let target = types.reference(types.slice);
    let source = types.source;
    let mut f = Fixture::new(types, &[source]);
    let first = f.transmute(f.args[0], f.target);
    let data = f.node(Opcode::Field, &[first], 0, f.thin);
    let metadata = f.metadata(first);
    let middle = f.pointer(data, metadata, middle);
    f.fixed(Opcode::RequireDefined, &[middle], 0, None);
    let second = f.transmute(middle, target);
    f.fixed(Opcode::RequireDefined, &[second], 0, None);
    let len = f.metadata(second);
    f.ret(len);
    verify_graph(&mut f.graph).unwrap();
    let original = f.graph.clone();
    f.optimize();
    assert_eq!(converted_metadata(&f.graph), if enabled() { 0 } else { 2 });
    assert_eq!(count(&f.graph, Opcode::TransmuteValue), 2);
    assert_eq!(count(&f.graph, Opcode::RequireDefined), 2);
    for len in [0, 1, 4, 255, 256, u64::MAX] {
        let args = [value(0x20000, len)];
        assert_eq!(
            run(&original, &args),
            Ok(SemanticOutcome::Return(Value::Scalar(len)))
        );
        assert_eq!(run(&f.graph, &args), run(&original, &args));
    }
}

#[test]
fn source_poison_still_fails_between_original_effects_after_length_becomes_constant() {
    let types = Types::new();
    let mut f = Fixture::new(types, &[]);
    f.call("before");
    let data = f.node(Opcode::Undefined, &[], 0, f.thin);
    let len = f.graph.add_constant(7, f.word);
    let source = f.pointer(data, len, f.source);
    let converted = f.transmute(source, f.target);
    let result = f.metadata(converted);
    f.call("after");
    f.ret(result);
    verify_graph(&mut f.graph).unwrap();
    for optimized in [false, true] {
        if optimized {
            f.optimize();
        }
        let mut events = Vec::new();
        let result =
            Interpreter::new(&f.graph).run_values(&[], &mut |symbol: &str, _: &[Value]| {
                events.push(symbol.to_owned());
                Ok(Value::Scalar(0))
            });
        assert!(
            matches!(result, Err(InterpretError::PoisonObserved(_))),
            "{result:?}"
        );
        assert_eq!(events, ["before"]);
        assert_eq!(count(&f.graph, Opcode::TransmuteValue), 1);
    }
    assert_eq!(converted_metadata(&f.graph), usize::from(!enabled()));
}

#[test]
fn target_nonnull_decode_still_fails_after_metadata_projection_is_replaced() {
    let mut types = Types::new();
    let target = types.reference(types.string);
    let mut f = Fixture::new(types, &[]);
    f.call("before");
    let data = f.graph.add_constant(0, f.thin);
    let len = f.graph.add_constant(9, f.word);
    let source = f.pointer(data, len, f.source);
    let converted = f.transmute(source, target);
    let result = f.metadata(converted);
    f.call("after");
    f.ret(result);
    verify_graph(&mut f.graph).unwrap();
    for optimized in [false, true] {
        if optimized {
            f.optimize();
        }
        let mut events = Vec::new();
        let result =
            Interpreter::new(&f.graph).run_values(&[], &mut |symbol: &str, _: &[Value]| {
                events.push(symbol.to_owned());
                Ok(Value::Scalar(0))
            });
        assert!(
            matches!(result, Err(InterpretError::UndefinedBehaviour(_))),
            "{result:?}"
        );
        assert_eq!(events, ["before"]);
        assert_eq!(count(&f.graph, Opcode::TransmuteValue), 1);
    }
    assert_eq!(converted_metadata(&f.graph), usize::from(!enabled()));
}

#[test]
fn an_unexecuted_branch_does_not_observe_its_poison_source() {
    let mut types = Types::new();
    let boolean = types
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut f = Fixture::new(types, &[boolean]);
    let effect = f.effect;
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[f.args[0]],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let yes = project(&mut f.graph, branch, 0, ValueType::Control);
    let no = project(&mut f.graph, branch, 1, ValueType::Control);
    f.control = yes;
    let data = f.node(Opcode::Undefined, &[], 0, f.thin);
    let len = f.graph.add_constant(7, f.word);
    let source = f.pointer(data, len, f.source);
    let converted = f.transmute(source, f.target);
    let result = f.metadata(converted);
    f.ret(result);
    f.control = no;
    f.effect = effect;
    let fallback = f.graph.add_constant(13, f.word);
    f.ret(fallback);
    verify_graph(&mut f.graph).unwrap();
    for optimized in [false, true] {
        if optimized {
            f.optimize();
        }
        assert_eq!(
            run(&f.graph, &[Value::Scalar(0)]),
            Ok(SemanticOutcome::Return(Value::Scalar(13)))
        );
        assert!(matches!(
            run(&f.graph, &[Value::Scalar(1)]),
            Err(InterpretError::PoisonObserved(_))
        ));
    }
}

#[test]
fn a_loaded_wide_pointer_keeps_its_snapshot_after_aliased_storage_is_overwritten() {
    let mut types = Types::new();
    let address_ty = types.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: types.source,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut f = Fixture::new(types, &[address_ty]);
    let source = f
        .fixed(Opcode::LoadValue, &[f.args[0]], 0, Some(f.source))
        .unwrap();
    let data = f.graph.add_constant(0x30000, f.thin);
    let new_len = f.graph.add_constant(99, f.word);
    let replacement = f.pointer(data, new_len, f.source);
    f.fixed(Opcode::StoreValue, &[f.args[0], replacement], 0, None);
    let converted = f.transmute(source, f.target);
    let result = f.metadata(converted);
    f.ret(result);
    verify_graph(&mut f.graph).unwrap();
    let original = f.graph.clone();
    let mut memory = SemanticMemory::new();
    let address = memory.allocate(16, 8, true).unwrap();
    memory
        .store(&f.graph.oracle, address, f.source, &value(0x20000, 4))
        .unwrap();
    f.optimize();
    assert_eq!(converted_metadata(&f.graph), usize::from(!enabled()));
    assert_eq!(count(&f.graph, Opcode::LoadValue), 1, "{}", f.graph.dump());
    for graph in [&original, &f.graph] {
        let mut storage = memory.clone();
        let result = Interpreter::new(graph).run_values_in_memory(
            &[Value::Scalar(address)],
            &mut |symbol: &str, _: &[Value]| Err(InterpretError::UnknownCallee(symbol.into())),
            &mut storage,
        );
        assert_eq!(result, Ok(SemanticOutcome::Return(Value::Scalar(4))));
        assert_eq!(
            storage.load(&graph.oracle, address, f.source).unwrap(),
            value(0x30000, 99)
        );
    }
}

#[test]
fn matching_total_size_does_not_certify_swapped_overlapping_padded_or_missing_layouts() {
    for mode in [
        "swapped",
        "overlap",
        "padding",
        "missing-fields",
        "metadata-width",
        "restricted-metadata",
        "restricted-metadata-lane",
    ] {
        let mut types = Types::new();
        match mode {
            "swapped" => {
                let fields = &mut types
                    .oracle
                    .aggregates
                    .get_mut(&types.target)
                    .unwrap()
                    .fields;
                fields[0].offset = 8;
                fields[1].offset = 0;
            }
            "overlap" => {
                types
                    .oracle
                    .aggregates
                    .get_mut(&types.source)
                    .unwrap()
                    .fields[1]
                    .offset = 0;
                types
                    .oracle
                    .aggregates
                    .get_mut(&types.target)
                    .unwrap()
                    .fields[1]
                    .offset = 0;
            }
            "padding" => {
                // A missing physical lane byte is not an initialized field,
                // even though both enclosing wide pointers still occupy16B.
                types.oracle.layouts[types.thin.0 as usize] = Some(SizeAlign { size: 4, align: 4 });
            }
            "missing-fields" => {
                types.oracle.aggregates.remove(&types.target);
            }
            "metadata-width" => {
                types.oracle.layouts[types.word.0 as usize] = Some(SizeAlign { size: 4, align: 4 });
            }
            "restricted-metadata" => types.oracle.record_scalar_validity(
                types.word,
                ScalarValidity::Scalar(ScalarValidRange::new(64, 0, 255).unwrap()),
            ),
            "restricted-metadata-lane" => types.oracle.record_scalar_validity(
                types.target,
                ScalarValidity::Pair {
                    first: None,
                    second: Some(ScalarValidRange::new(64, 0, 255).unwrap()),
                    second_offset: 8,
                },
            ),
            _ => unreachable!(),
        }
        let source = types.source;
        let mut f = Fixture::new(types, &[source]);
        let converted = f.transmute(f.args[0], f.target);
        let result = f.metadata(converted);
        f.ret(result);
        f.optimize();
        assert_eq!(
            converted_metadata(&f.graph),
            1,
            "{mode}: {}",
            f.graph.dump()
        );
        assert_eq!(count(&f.graph, Opcode::TransmuteValue), 1, "{mode}");
    }
}

#[test]
fn vtable_plain_u64_and_ordinary_pair_are_not_usize_wide_pointer_certificates() {
    for mode in ["vtable", "u64", "tuple"] {
        let mut types = Types::new();
        let target = match mode {
            "vtable" => {
                let dynamic = types.oracle.intern(
                    TypeShape::Other {
                        name: "dyn Probe".into(),
                    },
                    None,
                );
                types.oracle.record_unsized(dynamic);
                wide(&mut types.oracle, dynamic, types.thin, types.thin)
            }
            "u64" => {
                let integer = types.oracle.intern(
                    TypeShape::Int {
                        width: IntegerWidth::W64,
                        signed: false,
                    },
                    Some(SizeAlign { size: 8, align: 8 }),
                );
                wide(&mut types.oracle, types.string, types.thin, integer)
            }
            "tuple" => {
                let ty = types.oracle.intern(
                    TypeShape::Tuple(vec![types.thin, types.word]),
                    Some(SizeAlign { size: 16, align: 8 }),
                );
                let layout = types.oracle.aggregates[&types.source].clone();
                types.oracle.record_aggregate(ty, layout);
                ty
            }
            _ => unreachable!(),
        };
        let source = types.source;
        let mut f = Fixture::new(types, &[source]);
        let converted = f.transmute(f.args[0], target);
        let result = f.metadata(converted);
        f.ret(result);
        f.optimize();
        assert_eq!(
            converted_metadata(&f.graph),
            1,
            "{mode}: {}",
            f.graph.dump()
        );
    }
}

#[test]
fn union_and_maybe_uninit_sources_do_not_authorize_metadata_forwarding() {
    for marked in [false, true] {
        let mut types = Types::new();
        let raw = types.oracle.intern(
            TypeShape::Adt {
                name: "RepresentationStorage".into(),
            },
            Some(SizeAlign { size: 16, align: 8 }),
        );
        types.oracle.record_union(
            raw,
            vec![FieldLayout {
                ty: types.source,
                offset: 0,
            }],
        );
        if marked {
            types.oracle.record_maybe_uninit(raw, types.source);
        }
        let mut f = Fixture::new(types, &[]);
        let source = f.node(Opcode::Undefined, &[], 0, raw);
        let converted = f.transmute(source, f.target);
        let result = f.metadata(converted);
        f.ret(result);
        if !marked {
            // Ordinary unions are outside S's typed value subset. They must
            // fail graph validation rather than reaching a field certificate.
            assert!(verify_graph(&mut f.graph).is_err());
            assert_eq!(converted_metadata(&f.graph), 1);
            continue;
        }
        verify_graph(&mut f.graph).unwrap();
        assert!(matches!(
            run(&f.graph, &[]),
            Err(InterpretError::PoisonObserved(_))
        ));
        f.optimize();
        assert_eq!(converted_metadata(&f.graph), 1);
        assert!(matches!(
            run(&f.graph, &[]),
            Err(InterpretError::PoisonObserved(_))
        ));
    }
}

#[test]
fn merged_dynamic_instances_are_not_used_as_metadata_certificates() {
    let types = Types::new();
    let source = types.source;
    let mut f = Fixture::new(types, &[source]);
    let jump = f
        .graph
        .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
    let region = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
    let merged = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, f.args[0]],
        0,
        ValueType::Rust(source),
    );
    f.control = region;
    let converted = f.transmute(merged, f.target);
    let result = f.metadata(converted);
    f.ret(result);
    f.optimize();
    assert_eq!(converted_metadata(&f.graph), 1);
    assert_eq!(
        run(&f.graph, &[value(0x20000, 17)]),
        Ok(SemanticOutcome::Return(Value::Scalar(17)))
    );
}

#[test]
fn broad_value_proof_bails_out_without_changing_fixed_semantics() {
    let types = Types::new();
    let word = types.word;
    let mut f = Fixture::new(types, &[word; 256]);
    let data = f.graph.add_constant(0x20000, f.thin);
    // Exceed the proof's total visit budget with a depth-eight tree. A deep
    // linear expression would also exercise the interpreter's recursive value
    // evaluation, which is independent of the iterative certificate budget.
    let mut values = f.args.clone();
    while values.len() > 1 {
        values = values
            .chunks_exact(2)
            .map(|pair| f.node(Opcode::Add, pair, 0, f.word))
            .collect();
    }
    let source = f.pointer(data, values[0], f.source);
    let converted = f.transmute(source, f.target);
    let result = f.metadata(converted);
    f.ret(result);
    verify_graph(&mut f.graph).unwrap();
    let original = f.graph.clone();
    f.optimize();
    assert_eq!(converted_metadata(&f.graph), 1);
    assert_eq!(count(&f.graph, Opcode::TransmuteValue), 1);
    let args: Vec<_> = (1..=256).map(Value::Scalar).collect();
    assert_eq!(
        run(&original, &args),
        Ok(SemanticOutcome::Return(Value::Scalar(32896)))
    );
    assert_eq!(run(&f.graph, &args), run(&original, &args));
}

#[test]
fn long_effect_proof_bails_out_and_keeps_every_fixed_event() {
    let types = Types::new();
    let mut f = Fixture::new(types, &[]);
    let mut expected: Vec<_> = (0..100).map(|index| format!("before_{index}")).collect();
    for event in &expected {
        f.call(event);
    }
    let data = f.graph.add_constant(0x20000, f.thin);
    let len = f.graph.add_constant(17, f.word);
    let source = f.pointer(data, len, f.source);
    let converted = f.transmute(source, f.target);
    let result = f.metadata(converted);
    f.call("after");
    expected.push("after".into());
    f.ret(result);
    verify_graph(&mut f.graph).unwrap();
    let original = f.graph.clone();
    f.optimize();
    assert_eq!(converted_metadata(&f.graph), 1);
    assert_eq!(count(&f.graph, Opcode::TransmuteValue), 1);
    assert_eq!(count(&f.graph, Opcode::Call), expected.len());
    for graph in [&original, &f.graph] {
        let mut events = Vec::new();
        let result = Interpreter::new(graph).run_values(&[], &mut |symbol: &str, _: &[Value]| {
            events.push(symbol.to_owned());
            Ok(Value::Scalar(0))
        });
        assert_eq!(result, Ok(SemanticOutcome::Return(Value::Scalar(17))));
        assert_eq!(events, expected);
    }
}
