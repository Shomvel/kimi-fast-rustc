use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes,
    SizeAlign, TyId, TypeOracle, TypeShape,
};
use kfrc_son::{
    CallTarget, FunctionGraph, GraphLevel, GraphState, InterpretError, Interpreter, NodeId, Opcode,
    SemanticOutcome, Value, ValueType, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    pair: TyId,
    enumeration: TyId,
    pointer: TyId,
    wide: TyId,
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
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pair = types.intern(
            TypeShape::Tuple(vec![word, word]),
            Some(SizeAlign { size: 16, align: 8 }),
        );
        types.record_aggregate(
            pair,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: word,
                        offset: 0,
                    },
                    FieldLayout {
                        ty: word,
                        offset: 8,
                    },
                ],
                variants: Vec::new(),
            },
        );
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "Choice".into(),
            },
            Some(SizeAlign { size: 16, align: 8 }),
        );
        types.record_aggregate(
            enumeration,
            AggregateLayout {
                fields: Vec::new(),
                variants: vec![
                    vec![],
                    vec![FieldLayout {
                        ty: word,
                        offset: 8,
                    }],
                ],
            },
        );
        types.record_semantic_enum(enumeration, vec![7, 91]);
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: word,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let slice = types.intern(TypeShape::Slice { element: word }, None);
        let wide = types.intern(
            TypeShape::Reference {
                mutability: Mutability::Not,
                pointee: slice,
            },
            Some(SizeAlign { size: 16, align: 8 }),
        );
        types.record_aggregate(
            wide,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: pointer,
                        offset: 0,
                    },
                    FieldLayout {
                        ty: word,
                        offset: 8,
                    },
                ],
                variants: Vec::new(),
            },
        );
        let mut graph = FunctionGraph::new(
            "semantic_verify",
            FunctionAbi {
                args: Vec::new(),
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
        Self {
            graph,
            word,
            pair,
            enumeration,
            pointer,
            wide,
            control,
            effect,
        }
    }

    fn node(&mut self, op: Opcode, inputs: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, inputs, payload, ValueType::Rust(ty))
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

    fn run(&mut self) -> Result<SemanticOutcome, InterpretError> {
        verify_graph(&mut self.graph).unwrap();
        Interpreter::new(&self.graph).run_values(&[], &mut |name: &str, _: &[Value]| {
            Err(InterpretError::UnknownCallee(name.into()))
        })
    }
}

#[test]
fn partial_initialization_only_poisoned_leaf_is_unobservable() {
    let mut f = Fixture::new();
    let uninit = f.node(Opcode::Undefined, &[], 0, f.pair);
    let seven = f.graph.add_constant(7, f.word);
    let updated = f.node(Opcode::InsertField, &[uninit, seven], 0, f.pair);
    let field = f.node(Opcode::Field, &[updated], 0, f.word);
    f.ret(field);
    assert_eq!(f.run().unwrap(), SemanticOutcome::Return(Value::Scalar(7)));

    let return_node = f
        .graph
        .ids()
        .find(|node| f.graph.op(*node) == Opcode::Return)
        .unwrap();
    let other = f.node(Opcode::Field, &[updated], 1, f.word);
    f.graph.set_input(return_node, 1, other);
    assert!(matches!(f.run(), Err(InterpretError::PoisonObserved(_))));
}

#[test]
fn aggregate_insert_does_not_mutate_earlier_value_snapshot() {
    let mut f = Fixture::new();
    let a = f.graph.add_constant(11, f.word);
    let b = f.graph.add_constant(23, f.word);
    let original = f.node(Opcode::Aggregate, &[a, b], 0, f.pair);
    let updated = f.node(Opcode::InsertField, &[original, b], 0, f.pair);
    let old_field = f.node(Opcode::Field, &[original], 0, f.word);
    let new_field = f.node(Opcode::Field, &[updated], 0, f.word);
    let difference = f.node(Opcode::Sub, &[new_field, old_field], 0, f.word);
    f.ret(difference);
    assert_eq!(f.run().unwrap(), SemanticOutcome::Return(Value::Scalar(12)));
}

#[test]
fn enum_queries_use_logical_variant_and_source_discriminant() {
    for (op, expected) in [(Opcode::VariantOf, 1), (Opcode::Discriminant, 91)] {
        let mut f = Fixture::new();
        let poison = f.node(Opcode::Undefined, &[], 0, f.word);
        let variant = f.node(Opcode::MakeVariant, &[poison], 1, f.enumeration);
        let query = f.node(op, &[variant], 0, f.word);
        f.ret(query);
        assert_eq!(
            f.run().unwrap(),
            SemanticOutcome::Return(Value::Scalar(expected))
        );
    }
}

#[test]
fn inactive_variant_field_is_poison_without_interpreting_payload() {
    let mut f = Fixture::new();
    let variant = f.node(Opcode::MakeVariant, &[], 0, f.enumeration);
    let field_payload = f.graph.intern_variant_field(1, 0);
    let field = f.node(Opcode::VariantField, &[variant], field_payload, f.word);
    f.ret(field);
    assert!(matches!(f.run(), Err(InterpretError::PoisonObserved(_))));
}

#[test]
fn wide_pointer_parts_are_values_and_do_not_require_memory() {
    let mut f = Fixture::new();
    let pointer = f.graph.add_constant(0x2000, f.pointer);
    let length = f.graph.add_constant(17, f.word);
    let wide = f.node(Opcode::WidePtr, &[pointer, length], 0, f.wide);
    let result = f.node(Opcode::Field, &[wide], 1, f.word);
    f.ret(result);
    assert_eq!(f.run().unwrap(), SemanticOutcome::Return(Value::Scalar(17)));
}

#[test]
fn semantic_call_observes_whole_argument_and_returns_a_value_snapshot() {
    let mut f = Fixture::new();
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    let pair = f.node(Opcode::Aggregate, &[one, two], 0, f.pair);
    let target = f.graph.add_callee(CallTarget {
        symbol: "swap".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Pair],
            ret: PassMode::Pair,
            c_variadic: false,
        },
    });
    let call = f.graph.add_node(
        Opcode::Call,
        f.control,
        &[f.effect, pair],
        target,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(f.pair),
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
    let result = f.node(Opcode::Projection, &[call], 2, f.pair);
    let first = f.node(Opcode::Field, &[result], 0, f.word);
    f.ret(first);
    verify_graph(&mut f.graph).unwrap();
    let mut seen = 0;
    let result = Interpreter::new(&f.graph)
        .run_values(&[], &mut |name: &str, args: &[Value]| {
            assert_eq!(name, "swap");
            assert_eq!(
                args,
                &[Value::Aggregate(vec![Value::Scalar(1), Value::Scalar(2)])]
            );
            seen += 1;
            Ok(Value::Aggregate(vec![Value::Scalar(2), Value::Scalar(1)]))
        })
        .unwrap();
    assert_eq!(seen, 1);
    assert_eq!(result, SemanticOutcome::Return(Value::Scalar(2)));

    let poison = f.node(Opcode::Undefined, &[], 0, f.word);
    f.graph.set_input(pair, 1, poison);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        Interpreter::new(&f.graph).run_values(&[], &mut |_: &str, _: &[Value]| {
            panic!("poison argument must not reach the call handler")
        }),
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn semantic_verifier_rejects_bad_fields_layout_addresses_and_unimplemented_r() {
    let mut f = Fixture::new();
    let zero = f.graph.add_constant(0, f.word);
    let pair = f.node(Opcode::Aggregate, &[zero, zero], 0, f.pair);
    let bad = f.node(Opcode::Field, &[pair], 2, f.word);
    f.ret(bad);
    assert!(verify_graph(&mut f.graph).is_err());
    f.graph.set_payload(bad, 0);
    verify_graph(&mut f.graph).unwrap();
    f.graph.level = GraphLevel::Rust;
    assert!(verify_graph(&mut f.graph).is_err());
    assert_ne!(f.graph.state, GraphState::Verified);
    f.graph.level = GraphLevel::Layout;
    assert!(verify_graph(&mut f.graph).is_err());

    let mut f = Fixture::new();
    let pointer = f.graph.add_constant(0x1000, f.pointer);
    let zero = f.graph.add_constant(0, f.word);
    let fake_aggregate = f.node(Opcode::PtrOffset, &[pointer, zero], 0, f.pair);
    f.ret(fake_aggregate);
    assert!(verify_graph(&mut f.graph).is_err());
}

#[test]
fn interpreter_apis_do_not_silently_cross_representation_boundaries() {
    let mut f = Fixture::new();
    let value = f.graph.add_constant(1, f.word);
    f.ret(value);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        Interpreter::new(&f.graph).run(&[], &mut |_: &str, _: &[u64]| Ok(0)),
        Err(InterpretError::Malformed(_))
    ));
    f.graph.level = GraphLevel::Layout;
    assert!(matches!(
        Interpreter::new(&f.graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| Ok(Value::Scalar(0))),
        Err(InterpretError::Malformed(_))
    ));
}

impl Fixture {
    fn slot(&mut self, ty: TyId, local: u32) -> NodeId {
        let pointer = self.graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: ty,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        self.node(Opcode::LocalSlot, &[], local, pointer)
    }

    fn fixed_memory(
        &mut self,
        op: Opcode,
        operands: &[NodeId],
        result: Option<TyId>,
    ) -> Option<NodeId> {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(operands);
        let mut results = vec![ValueType::Control, ValueType::Effect];
        if let Some(ty) = result {
            results.push(ValueType::Rust(ty));
        }
        let fixed = self
            .graph
            .add_node(op, self.control, &inputs, 0, ValueType::Tuple(results));
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[fixed],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[fixed],
            1,
            ValueType::Effect,
        );
        result.map(|ty| self.node(Opcode::Projection, &[fixed], 2, ty))
    }
}

#[test]
fn semantic_memory_loads_own_snapshots_and_keep_uninitialized_fields_local() {
    let mut f = Fixture::new();
    let slot = f.slot(f.pair, 0);
    let pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: f.word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let field = f.node(Opcode::FieldAddr, &[slot], 0, pointer);
    let one = f.graph.add_constant(1, f.word);
    f.fixed_memory(Opcode::StoreValue, &[field, one], None);
    let snapshot = f
        .fixed_memory(Opcode::LoadValue, &[slot], Some(f.pair))
        .unwrap();
    let two = f.graph.add_constant(2, f.word);
    f.fixed_memory(Opcode::StoreValue, &[field, two], None);
    let old = f.node(Opcode::Field, &[snapshot], 0, f.word);
    f.ret(old);
    assert_eq!(f.run().unwrap(), SemanticOutcome::Return(Value::Scalar(1)));
}

#[test]
fn semantic_memory_lifetime_and_immutable_storage_are_checked() {
    let mut f = Fixture::new();
    let slot = f.slot(f.word, 0);
    let one = f.graph.add_constant(1, f.word);
    f.fixed_memory(Opcode::StoreValue, &[slot, one], None);
    f.fixed_memory(Opcode::SlotDead, &[slot], None);
    let loaded = f
        .fixed_memory(Opcode::LoadValue, &[slot], Some(f.word))
        .unwrap();
    f.ret(loaded);
    assert!(matches!(
        f.run(),
        Err(InterpretError::UndefinedBehaviour(_))
    ));

    let mut memory = kfrc_son::interpret::SemanticMemory::new();
    let address = memory.allocate(8, 8, false).unwrap();
    assert!(matches!(
        memory.store(&f.graph.oracle, address, f.word, &Value::Scalar(1)),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
}

#[test]
fn enum_memory_only_decodes_active_payload_and_set_variant_preserves_written_fields() {
    let mut f = Fixture::new();
    f.graph.oracle.enums.insert(
        f.enumeration,
        kfrc_body::EnumLayout {
            tag: Some(FieldLayout {
                ty: f.word,
                offset: 0,
            }),
            encoding: kfrc_body::TagEncoding::Direct {
                discriminants: vec![7, 91],
            },
        },
    );
    let slot = f.slot(f.enumeration, 0);
    let pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: f.word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let payload = f.graph.intern_variant_field(1, 0);
    let address = f.node(Opcode::VariantFieldAddr, &[slot], payload, pointer);
    let forty_two = f.graph.add_constant(42, f.word);
    f.fixed_memory(Opcode::StoreValue, &[address, forty_two], None);
    let variant = f.graph.add_constant(1, f.word);
    f.fixed_memory(Opcode::SetVariant, &[slot, variant], None);
    let loaded = f
        .fixed_memory(Opcode::LoadValue, &[slot], Some(f.enumeration))
        .unwrap();
    let field = f.node(Opcode::VariantField, &[loaded], payload, f.word);
    f.ret(field);
    assert_eq!(f.run().unwrap(), SemanticOutcome::Return(Value::Scalar(42)));

    let mut memory = kfrc_son::interpret::SemanticMemory::new();
    let address = memory.allocate(16, 8, true).unwrap();
    memory
        .store(
            &f.graph.oracle,
            address,
            f.enumeration,
            &Value::Variant {
                variant: 0,
                fields: vec![],
            },
        )
        .unwrap();
    assert_eq!(
        memory
            .load(&f.graph.oracle, address, f.enumeration)
            .unwrap(),
        Value::Variant {
            variant: 0,
            fields: vec![]
        }
    );
}

#[test]
fn copying_memory_preserves_partial_initialization_and_rejects_overlap() {
    let mut f = Fixture::new();
    let source = f.slot(f.pair, 0);
    let destination = f.slot(f.pair, 1);
    let one = f.graph.add_constant(1, f.word);
    let undefined = f.node(Opcode::Undefined, &[], 0, f.word);
    let pair = f.node(Opcode::Aggregate, &[one, undefined], 0, f.pair);
    f.fixed_memory(Opcode::StoreValue, &[source, pair], None);
    f.fixed_memory(
        Opcode::CopyNonOverlapping,
        &[source, destination, one],
        None,
    );
    let copy = f
        .fixed_memory(Opcode::LoadValue, &[destination], Some(f.pair))
        .unwrap();
    let field = f.node(Opcode::Field, &[copy], 0, f.word);
    f.ret(field);
    assert_eq!(f.run().unwrap(), SemanticOutcome::Return(Value::Scalar(1)));
    let copy_node = f
        .graph
        .ids()
        .find(|node| f.graph.op(*node) == Opcode::CopyNonOverlapping)
        .unwrap();
    f.graph.set_input(copy_node, 2, source);
    assert!(matches!(
        f.run(),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
}

#[test]
fn local_partition_requires_actual_nonescaping_storage() {
    let mut f = Fixture::new();
    let slot = f.slot(f.word, 7);
    let one = f.graph.add_constant(1, f.word);
    f.fixed_memory(Opcode::StoreValue, &[slot, one], None);
    let loaded = f
        .fixed_memory(Opcode::LoadValue, &[slot], Some(f.word))
        .unwrap();
    f.ret(loaded);
    f.graph
        .memory_partitions
        .push(kfrc_son::MemoryPartition::Local { local: 7 });
    let load = f.graph.input(loaded, 0);
    f.graph.set_payload(load, 1);
    verify_graph(&mut f.graph).unwrap();
    let ret = f
        .graph
        .ids()
        .find(|node| f.graph.op(*node) == Opcode::Return)
        .unwrap();
    f.graph.set_input(ret, 1, slot);
    assert!(verify_graph(&mut f.graph).is_err());
}

#[test]
fn transmute_reads_padding_as_uninitialized_and_observation_preserves_poison() {
    let mut f = Fixture::new();
    let byte = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let padded = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "Padded".into(),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    f.graph.oracle.record_aggregate(
        padded,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: byte,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    let one = f.graph.add_constant(1, byte);
    let aggregate = f.node(Opcode::Aggregate, &[one], 0, padded);
    let result = f
        .fixed_memory(Opcode::TransmuteValue, &[aggregate], Some(f.word))
        .unwrap();
    f.ret(result);
    assert!(matches!(f.run(), Err(InterpretError::PoisonObserved(_))));

    let mut f = Fixture::new();
    let poison = f.node(Opcode::Undefined, &[], 0, f.word);
    f.fixed_memory(Opcode::RequireDefined, &[poison], None);
    let zero = f.graph.add_constant(0, f.word);
    // No result is produced, but the fixed requirement must still execute.
    f.ret(zero);
    assert!(matches!(f.run(), Err(InterpretError::PoisonObserved(_))));
}

#[test]
fn unknown_call_can_mutate_borrowed_memory_without_changing_prior_load_snapshot() {
    struct Mutator {
        types: RecordedTypes,
        ty: TyId,
    }
    impl kfrc_son::SemanticCallHandler for Mutator {
        fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, InterpretError> {
            panic!("memory entry must be used")
        }
        fn call_with_memory(
            &mut self,
            symbol: &str,
            arguments: &[Value],
            memory: &mut kfrc_son::interpret::SemanticMemory,
        ) -> Result<Value, InterpretError> {
            assert_eq!(symbol, "mutate");
            let Value::Scalar(address) = arguments[0] else {
                panic!("pointer expected")
            };
            memory.store(&self.types, address, self.ty, &Value::Scalar(9))?;
            Ok(Value::Aggregate(vec![]))
        }
    }
    let mut f = Fixture::new();
    let slot = f.slot(f.word, 0);
    let one = f.graph.add_constant(1, f.word);
    f.fixed_memory(Opcode::StoreValue, &[slot, one], None);
    let snapshot = f
        .fixed_memory(Opcode::LoadValue, &[slot], Some(f.word))
        .unwrap();
    let unit = f
        .graph
        .oracle
        .intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let callee = f.graph.add_callee(CallTarget {
        symbol: "mutate".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Ignore,
            c_variadic: false,
        },
    });
    let call = f.graph.add_node(
        Opcode::Call,
        f.control,
        &[f.effect, slot],
        callee,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(unit),
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
    let current = f
        .fixed_memory(Opcode::LoadValue, &[slot], Some(f.word))
        .unwrap();
    let difference = f.node(Opcode::Sub, &[current, snapshot], 0, f.word);
    f.ret(difference);
    verify_graph(&mut f.graph).unwrap();
    let mut calls = Mutator {
        types: f.graph.oracle.clone(),
        ty: f.word,
    };
    assert_eq!(
        Interpreter::new(&f.graph)
            .run_values(&[], &mut calls)
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(8))
    );
}

#[test]
fn layout_return_observes_typed_fields_but_not_uninitialized_padding() {
    let mut f = Fixture::new();
    let byte = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let padded = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "PaddedReturn".into(),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    f.graph.oracle.record_aggregate(
        padded,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: byte,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    let one = f.graph.add_constant(1, byte);
    let aggregate = f.node(Opcode::Aggregate, &[one], 0, padded);
    f.ret(aggregate);
    assert_eq!(
        f.run().unwrap(),
        SemanticOutcome::Return(Value::Aggregate(vec![Value::Scalar(1)]))
    );
    kfrc_son::lower_layout(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(
        Interpreter::new(&f.graph)
            .run(&[], &mut |_: &str, _: &[u64]| unreachable!())
            .unwrap(),
        kfrc_son::Outcome::Return(1)
    );
}

#[test]
fn returning_a_stack_pointer_does_not_extend_the_callee_storage_lifetime() {
    let mut f = Fixture::new();
    let slot = f.slot(f.word, 0);
    let one = f.graph.add_constant(1, f.word);
    f.fixed_memory(Opcode::StoreValue, &[slot, one], None);
    f.ret(slot);
    verify_graph(&mut f.graph).unwrap();
    let mut memory = kfrc_son::interpret::SemanticMemory::new();
    let outcome = Interpreter::new(&f.graph)
        .run_values_in_memory(&[], &mut |_: &str, _: &[Value]| unreachable!(), &mut memory)
        .unwrap();
    let SemanticOutcome::Return(Value::Scalar(address)) = outcome else {
        panic!("expected returned pointer")
    };
    assert!(matches!(
        memory.load(&f.graph.oracle, address, f.word),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
}

#[test]
fn const_partition_requires_the_actual_immutable_data_allocation() {
    let mut f = Fixture::new();
    f.graph.symbols.push(("constant".into(), 0));
    f.graph.data_objects.push(kfrc_body::DataObject {
        symbol: "constant".into(),
        bytes: 42u64.to_le_bytes().to_vec(),
        align: 8,
        writable: false,
        ..Default::default()
    });
    f.graph
        .memory_partitions
        .push(kfrc_son::MemoryPartition::Const { symbol: 0 });
    let address = f.node(Opcode::SymbolAddress, &[], 0, f.pointer);
    let loaded = f
        .fixed_memory(Opcode::LoadValue, &[address], Some(f.word))
        .unwrap();
    let load = f.graph.input(loaded, 0);
    f.graph.set_payload(load, 1);
    f.ret(loaded);
    assert_eq!(f.run().unwrap(), SemanticOutcome::Return(Value::Scalar(42)));
    f.graph.data_objects[0].writable = true;
    assert!(verify_graph(&mut f.graph).is_err());
}

#[test]
fn shared_program_data_keeps_identity_and_mutations_across_frames() {
    let mut f = Fixture::new();
    f.graph.symbols.push(("counter".into(), 0));
    f.graph.data_objects.push(kfrc_body::DataObject {
        symbol: "counter".into(),
        bytes: 0u64.to_le_bytes().to_vec(),
        align: 8,
        writable: true,
        ..Default::default()
    });
    let address = f.node(Opcode::SymbolAddress, &[], 0, f.pointer);
    let old = f
        .fixed_memory(Opcode::LoadValue, &[address], Some(f.word))
        .unwrap();
    let one = f.graph.add_constant(1, f.word);
    let new = f.node(Opcode::Add, &[old, one], 0, f.word);
    f.fixed_memory(Opcode::StoreValue, &[address, new], None);
    f.ret(new);
    verify_graph(&mut f.graph).unwrap();
    let mut memory = kfrc_son::SemanticMemory::new();
    for expected in [1, 2] {
        assert_eq!(
            Interpreter::new(&f.graph)
                .run_values_in_memory(&[], &mut |_: &str, _: &[Value]| unreachable!(), &mut memory)
                .unwrap(),
            SemanticOutcome::Return(Value::Scalar(expected))
        );
    }
}

#[test]
fn missing_wide_metadata_cannot_turn_dst_rust_pointers_into_thin_addresses() {
    for kind in ["slice", "str", "struct"] {
        for reference in [false, true] {
            let mut f = Fixture::new();
            // Fixture already owns a valid &[u64]. Use a distinct pointee so
            // interning cannot reuse its real wide layout and field schema.
            let element = f.graph.oracle.intern(
                TypeShape::Int {
                    width: IntegerWidth::W16,
                    signed: false,
                },
                Some(SizeAlign { size: 2, align: 2 }),
            );
            let pointee = match kind {
                "slice" => f.graph.oracle.intern(TypeShape::Slice { element }, None),
                "str" => f.graph.oracle.intern(TypeShape::Str, None),
                _ => {
                    let ty = f.graph.oracle.intern(
                        TypeShape::Adt {
                            name: "UnsizedOwner".into(),
                        },
                        None,
                    );
                    f.graph.oracle.record_unsized(ty);
                    ty
                }
            };
            let shape = if reference {
                TypeShape::Reference {
                    mutability: Mutability::Not,
                    pointee,
                }
            } else {
                TypeShape::RawPointer {
                    mutability: Mutability::Mut,
                    pointee,
                }
            };
            let false_thin = f
                .graph
                .oracle
                .intern(shape, Some(SizeAlign { size: 8, align: 8 }));
            assert_eq!(
                f.graph.oracle.size_align(false_thin),
                Some(SizeAlign { size: 8, align: 8 })
            );
            assert_eq!(
                f.graph.oracle.semantic().field_count(false_thin, None),
                None
            );
            assert_eq!(f.graph.oracle.semantic().thin_pointee(false_thin), None);
            let value = f.node(Opcode::Undefined, &[], 0, false_thin);
            f.ret(value);
            assert!(verify_graph(&mut f.graph).is_err());
        }
    }
}

#[test]
fn place_addresses_keep_unsized_owners_without_becoming_wide_rust_values() {
    let mut f = Fixture::new();
    let string = f.graph.oracle.intern(TypeShape::Str, None);
    let byte = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let address = f
        .graph
        .oracle
        .intern(TypeShape::PlaceAddress { pointee: string }, None);
    let byte_address = f
        .graph
        .oracle
        .intern(TypeShape::PlaceAddress { pointee: byte }, None);
    let raw = f.graph.add_constant(0x1000, f.pointer);
    let place = f.node(Opcode::PointerCast, &[raw], 0, address);
    let index = f.graph.add_constant(3, f.word);
    let element = f.node(Opcode::IndexAddr, &[place, index], 0, byte_address);
    let result = f.node(Opcode::PointerCast, &[element], 0, f.pointer);
    f.ret(result);
    assert_eq!(f.run(), Ok(SemanticOutcome::Return(Value::Scalar(0x1003))));
    assert_eq!(
        f.graph.oracle.semantic().thin_pointee(address),
        Some(string)
    );
    assert!(!f.graph.oracle.semantic().is_product(address));
}

#[test]
fn index_address_distinguishes_pointer_steps_from_immediate_place_elements() {
    for case in 0..10 {
        let mut f = Fixture::new();
        let byte = f.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let array = f.graph.oracle.intern(
            TypeShape::Array {
                element: byte,
                length: 24,
            },
            Some(SizeAlign { size: 24, align: 1 }),
        );
        let outer = f.graph.oracle.intern(
            TypeShape::Array {
                element: array,
                length: 2,
            },
            Some(SizeAlign { size: 48, align: 1 }),
        );
        let slice = f
            .graph
            .oracle
            .intern(TypeShape::Slice { element: array }, None);
        let string = f.graph.oracle.intern(TypeShape::Str, None);
        let (source, result, stride) = match case {
            0 => (outer, outer, Some(48)), // Advance by one entire nested array.
            1 => (outer, array, Some(24)), // Select one immediate element.
            2 => (array, array, Some(24)), // The iterator<[u8; 24]> case.
            3 => (array, byte, Some(1)),
            4 => (slice, array, Some(24)),
            5 => (string, byte, Some(1)),
            6 => (outer, byte, None), // Cannot skip a level of array nesting.
            7 => (array, f.word, None), // Cannot choose an unrelated stride.
            8 => (slice, slice, None), // Unsized elements have no stride.
            _ => (string, string, None),
        };
        let source_address = f
            .graph
            .oracle
            .intern(TypeShape::PlaceAddress { pointee: source }, None);
        let result_address = f
            .graph
            .oracle
            .intern(TypeShape::PlaceAddress { pointee: result }, None);
        let raw = f.graph.add_constant(0x1000, f.pointer);
        let base = f.node(Opcode::PointerCast, &[raw], 0, source_address);
        // Computing a one-past address is valid; IndexAddr itself is not a
        // bounds assertion or a load. Existing memory operations enforce bounds.
        let index = f.graph.add_constant(2, f.word);
        let address = f.node(Opcode::IndexAddr, &[base, index], 0, result_address);
        let result = f.node(Opcode::PointerCast, &[address], 0, f.pointer);
        f.ret(result);
        if let Some(stride) = stride {
            let expected = 0x1000 + 2 * stride;
            assert_eq!(
                f.run(),
                Ok(SemanticOutcome::Return(Value::Scalar(expected)))
            );
            kfrc_son::lower_layout(&mut f.graph).unwrap();
            verify_graph(&mut f.graph).unwrap();
            assert_eq!(
                Interpreter::new(&f.graph)
                    .run(&[], &mut |_: &str, _: &[u64]| unreachable!())
                    .unwrap(),
                kfrc_son::Outcome::Return(expected),
                "index relation case {case}",
            );
        } else {
            assert!(
                verify_graph(&mut f.graph).is_err(),
                "index relation case {case}"
            );
        }
    }
}

#[test]
fn unsized_place_cannot_allocate_or_load_a_whole_value() {
    for allocation in [false, true] {
        let mut f = Fixture::new();
        let slice = f
            .graph
            .oracle
            .intern(TypeShape::Slice { element: f.word }, None);
        let address = f
            .graph
            .oracle
            .intern(TypeShape::PlaceAddress { pointee: slice }, None);
        let raw = f.graph.add_constant(0x1000, f.pointer);
        let place = if allocation {
            f.node(Opcode::LocalSlot, &[], 0, address)
        } else {
            f.node(Opcode::PointerCast, &[raw], 0, address)
        };
        if !allocation {
            let load = f.graph.add_node(
                Opcode::LoadValue,
                f.control,
                &[f.effect, place],
                0,
                ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(slice),
                ]),
            );
            f.control = f.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[load],
                0,
                ValueType::Control,
            );
            f.effect = f.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[load],
                1,
                ValueType::Effect,
            );
        }
        let zero = f.graph.add_constant(0, f.word);
        f.ret(zero);
        assert!(verify_graph(&mut f.graph).is_err());
    }
}

#[test]
fn extern_types_are_unsized_with_genuine_thin_pointer_metadata() {
    let mut f = Fixture::new();
    let foreign = f.graph.oracle.intern(
        TypeShape::Other {
            name: "ForeignType".into(),
        },
        None,
    );
    f.graph.oracle.record_thin_unsized(foreign);
    assert!(f.graph.oracle.semantic().is_unsized(foreign));
    assert!(!f.graph.oracle.semantic().requires_pointer_metadata(foreign));
    let pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: foreign,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let raw = f.graph.add_constant(0x1234, pointer);
    f.ret(raw);
    assert_eq!(f.run(), Ok(SemanticOutcome::Return(Value::Scalar(0x1234))));
}

#[test]
fn verification_rechecks_recursive_types_after_oracle_edits() {
    for start_at_second_member in [false, true] {
        let mut f = Fixture::new();
        let a = f.graph.oracle.intern(
            TypeShape::Adt { name: "A".into() },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let b = f.graph.oracle.intern(
            TypeShape::Adt { name: "B".into() },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut pointers = Vec::new();
        for pointee in [a, b] {
            pointers.push(f.graph.oracle.intern(
                TypeShape::RawPointer {
                    mutability: Mutability::Not,
                    pointee,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            ));
        }
        for (ty, field) in [(a, pointers[1]), (b, pointers[0])] {
            f.graph.oracle.record_semantic_aggregate(
                ty,
                kfrc_body::SemanticAggregate {
                    fields: vec![field],
                    variants: Vec::new(),
                },
            );
        }
        // Repeated values share the same recursive nominal types. Neither
        // cycle contains a type parameter until the producer fact changes.
        let root = pointers[usize::from(start_at_second_member)];
        let first = f.node(Opcode::Undefined, &[], 0, root);
        for _ in 0..128 {
            f.node(Opcode::Undefined, &[], 0, root);
        }
        let zero = f.graph.add_constant(0, f.word);
        f.ret(zero);
        verify_graph(&mut f.graph).unwrap();

        let parameter = f.graph.oracle.intern(
            TypeShape::Param {
                index: 0,
                name: "T".into(),
            },
            None,
        );
        f.graph.oracle.record_semantic_aggregate(
            a,
            kfrc_body::SemanticAggregate {
                // The cycle is deliberately traversed before the parameter.
                fields: vec![pointers[1], parameter],
                variants: Vec::new(),
            },
        );
        assert_eq!(
            verify_graph(&mut f.graph),
            Err(kfrc_son::VerifyError::TypeMismatch {
                node: first,
                detail: "type parameter remains outside R".into(),
            })
        );
        assert_ne!(f.graph.state, GraphState::Verified);
    }
}
