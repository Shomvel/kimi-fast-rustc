//! The MaybeUninit lang-item marker grants raw storage semantics, never T validity.
use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes,
    SizeAlign, TyId, TypeOracle, TypeShape,
};
use kfrc_son::{
    FunctionGraph, InterpretError, Interpreter, NodeId, Opcode, SemanticMemory, SemanticOutcome,
    Value, ValueType, verify_graph,
};
use std::sync::Arc;

fn raw(bytes: &[Option<u8>]) -> Value {
    Value::Uninit {
        bytes: Arc::new(bytes.to_vec()),
    }
}
fn scalar(types: &mut RecordedTypes, width: IntegerWidth) -> TyId {
    let size = u64::from(width.bits(64)) / 8;
    types.intern(
        TypeShape::Int {
            width,
            signed: false,
        },
        Some(SizeAlign { size, align: size }),
    )
}
fn wrapper(types: &mut RecordedTypes, inner: TyId, marked: bool) -> TyId {
    let layout = types.size_align(inner);
    let ty = types.intern(
        TypeShape::Adt {
            name: format!("core::mem::MaybeUninit<{}>", inner.0),
        },
        layout,
    );
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    types.record_union(
        ty,
        vec![
            FieldLayout {
                ty: unit,
                offset: 0,
            },
            FieldLayout {
                ty: inner,
                offset: 0,
            },
        ],
    );
    if marked {
        types.record_maybe_uninit(ty, inner);
    }
    ty
}
struct Fixture {
    graph: FunctionGraph,
    control: NodeId,
    effect: NodeId,
    args: Vec<NodeId>,
}
impl Fixture {
    fn new(types: RecordedTypes, args: &[TyId]) -> Self {
        let mut graph = FunctionGraph::new(
            "maybe_uninit",
            FunctionAbi {
                args: vec![PassMode::Direct; args.len()],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
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
        let args = args
            .iter()
            .enumerate()
            .map(|(i, ty)| {
                graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[start],
                    i as u32 + 2,
                    ValueType::Rust(*ty),
                )
            })
            .collect();
        Self {
            graph,
            control,
            effect,
            args,
        }
    }
    fn node(&mut self, op: Opcode, inputs: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, inputs, payload, ValueType::Rust(ty))
    }
    fn fixed(&mut self, op: Opcode, args: &[NodeId], ty: Option<TyId>) -> Option<NodeId> {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(args);
        let mut results = vec![ValueType::Control, ValueType::Effect];
        if let Some(ty) = ty {
            results.push(ValueType::Rust(ty));
        }
        let node = self
            .graph
            .add_node(op, self.control, &inputs, 0, ValueType::Tuple(results));
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            1,
            ValueType::Effect,
        );
        ty.map(|ty| self.node(Opcode::Projection, &[node], 2, ty))
    }
    fn transmute(&mut self, value: NodeId, ty: TyId) -> NodeId {
        self.fixed(Opcode::TransmuteValue, &[value], Some(ty))
            .unwrap()
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
    fn run(&mut self, args: &[Value]) -> Result<SemanticOutcome, InterpretError> {
        verify_graph(&mut self.graph).unwrap();
        Interpreter::new(&self.graph).run_values(args, &mut |_: &str, _: &[Value]| unreachable!())
    }
}

#[test]
fn uninitialized_container_is_a_safe_owned_argument_and_return() {
    let mut types = RecordedTypes::new();
    let word = scalar(&mut types, IntegerWidth::W64);
    let mu = wrapper(&mut types, word, true);
    let mut f = Fixture::new(types.clone(), &[]);
    let uninit = f.node(Opcode::Undefined, &[], 0, mu);
    f.ret(uninit);
    let value = raw(&[None; 8]);
    assert!(!value.contains_poison());
    assert_eq!(f.run(&[]), Ok(SemanticOutcome::Return(value.clone())));
    let mut identity = Fixture::new(types, &[mu]);
    identity.ret(identity.args[0]);
    assert_eq!(
        identity.run(&[value.clone()]),
        Ok(SemanticOutcome::Return(value))
    );
}

#[test]
fn invalid_bool_bytes_survive_container_memory_but_fail_typed_transmute() {
    let mut types = RecordedTypes::new();
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mu = wrapper(&mut types, boolean, true);
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: mu,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    for typed in [false, true] {
        let mut f = Fixture::new(types.clone(), &[mu]);
        let slot = f.node(Opcode::LocalSlot, &[], 0, pointer);
        f.fixed(Opcode::StoreValue, &[slot, f.args[0]], None);
        let snapshot = f.fixed(Opcode::LoadValue, &[slot], Some(mu)).unwrap();
        let result = if typed {
            f.transmute(snapshot, boolean)
        } else {
            snapshot
        };
        f.ret(result);
        let outcome = f.run(&[raw(&[Some(2)])]);
        if typed {
            assert!(matches!(
                outcome,
                Err(InterpretError::UndefinedBehaviour(_))
            ));
        } else {
            assert_eq!(outcome, Ok(SemanticOutcome::Return(raw(&[Some(2)]))));
        }
    }
}

#[test]
fn uninitialized_typed_reads_and_poison_laundering_are_rejected() {
    let mut types = RecordedTypes::new();
    let word = scalar(&mut types, IntegerWidth::W64);
    let mu = wrapper(&mut types, word, true);
    for (source, target) in [(mu, word), (word, mu)] {
        let mut f = Fixture::new(types.clone(), &[]);
        let value = f.node(Opcode::Undefined, &[], 0, source);
        let result = f.transmute(value, target);
        f.ret(result);
        assert!(matches!(f.run(&[]), Err(InterpretError::PoisonObserved(_))));
    }
}

#[test]
fn padding_and_partial_initialization_copy_without_decoding_and_snapshots_do_not_alias() {
    let mut types = RecordedTypes::new();
    let byte = scalar(&mut types, IntegerWidth::W8);
    let word = scalar(&mut types, IntegerWidth::W32);
    let padded = types.intern(
        TypeShape::Tuple(vec![byte, word]),
        Some(SizeAlign { size: 8, align: 4 }),
    );
    types.record_aggregate(
        padded,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: byte,
                    offset: 0,
                },
                FieldLayout {
                    ty: word,
                    offset: 4,
                },
            ],
            variants: vec![],
        },
    );
    let mu = wrapper(&mut types, padded, true);
    let mut memory = SemanticMemory::new();
    let source = memory.allocate(8, 4, true).unwrap();
    let dest = memory.allocate(8, 4, true).unwrap();
    let bytes = [
        Some(7),
        None,
        Some(0xa5),
        None,
        Some(9),
        None,
        Some(0),
        Some(0),
    ];
    memory.write_bytes(source, &bytes).unwrap();
    let snapshot = memory.load(&types, source, mu).unwrap();
    memory.store(&types, dest, mu, &snapshot.clone()).unwrap();
    memory.write_bytes(source, &[Some(0xff); 8]).unwrap();
    assert_eq!(snapshot, raw(&bytes));
    assert_eq!(memory.read_bytes(dest, 8).unwrap(), bytes);
    assert_eq!(memory.load(&types, dest, mu).unwrap(), snapshot);
    assert!(memory.load(&types, dest, padded).unwrap().contains_poison());

    let mut f = Fixture::new(types, &[]);
    let a = f.graph.add_constant(7, byte);
    let b = f.graph.add_constant(9, word);
    let value = f.node(Opcode::Aggregate, &[a, b], 0, padded);
    let value = f.transmute(value, mu);
    f.ret(value);
    assert_eq!(
        f.run(&[]),
        Ok(SemanticOutcome::Return(raw(&[
            Some(7),
            None,
            None,
            None,
            Some(9),
            Some(0),
            Some(0),
            Some(0)
        ])))
    );
}

#[test]
fn array_iter_style_nested_aggregate_keeps_maybe_uninit_elements() {
    let mut types = RecordedTypes::new();
    let byte = scalar(&mut types, IntegerWidth::W8);
    let mu = wrapper(&mut types, byte, true);
    let array = types.intern(
        TypeShape::Array {
            element: byte,
            length: 2,
        },
        Some(SizeAlign { size: 2, align: 1 }),
    );
    let mu_array = types.intern(
        TypeShape::Array {
            element: mu,
            length: 2,
        },
        Some(SizeAlign { size: 2, align: 1 }),
    );
    let outer = types.intern(
        TypeShape::Tuple(vec![byte, mu_array]),
        Some(SizeAlign { size: 3, align: 1 }),
    );
    types.record_aggregate(
        outer,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: byte,
                    offset: 0,
                },
                FieldLayout {
                    ty: mu_array,
                    offset: 1,
                },
            ],
            variants: vec![],
        },
    );
    let mut f = Fixture::new(types, &[]);
    let seven = f.graph.add_constant(7, byte);
    let nine = f.graph.add_constant(9, byte);
    let array = f.node(Opcode::Aggregate, &[seven, nine], 0, array);
    let data = f.transmute(array, mu_array);
    let iter = f.node(Opcode::Aggregate, &[seven, data], 0, outer);
    f.ret(iter);
    assert_eq!(
        f.run(&[]),
        Ok(SemanticOutcome::Return(Value::Aggregate(vec![
            Value::Scalar(7),
            Value::Aggregate(vec![raw(&[Some(7)]), raw(&[Some(9)])])
        ])))
    );
}

#[test]
fn malformed_opaque_arguments_and_general_union_are_not_accepted() {
    let mut types = RecordedTypes::new();
    let byte = scalar(&mut types, IntegerWidth::W8);
    let mu = wrapper(&mut types, byte, true);
    let mut f = Fixture::new(types.clone(), &[mu]);
    f.ret(f.args[0]);
    for bad in [
        raw(&[]),
        raw(&[Some(0), None]),
        Value::Scalar(0),
        Value::Aggregate(vec![]),
    ] {
        assert!(matches!(f.run(&[bad]), Err(InterpretError::Malformed(_))));
    }
    let mut scalar_identity = Fixture::new(types, &[byte]);
    scalar_identity.ret(scalar_identity.args[0]);
    assert!(matches!(
        scalar_identity.run(&[raw(&[Some(0)])]),
        Err(InterpretError::Malformed(_))
    ));
    let mut types = RecordedTypes::new();
    let byte = scalar(&mut types, IntegerWidth::W8);
    let fake_mu = wrapper(&mut types, byte, false);
    let mut f = Fixture::new(types, &[]);
    let value = f.node(Opcode::Undefined, &[], 0, fake_mu);
    f.ret(value);
    assert!(verify_graph(&mut f.graph).is_err());
}

#[test]
fn marker_does_not_turn_union_fields_into_a_product() {
    let mut types = RecordedTypes::new();
    let byte = scalar(&mut types, IntegerWidth::W8);
    let mu = wrapper(&mut types, byte, true);
    assert!(types.semantic().is_union(mu));
    assert!(!types.semantic().is_product(mu));
    assert_eq!(types.semantic().field_type(mu, None, 1), Some(byte));
    let mut f = Fixture::new(types, &[]);
    let value = f.node(Opcode::Undefined, &[], 0, mu);
    let field = f.node(Opcode::Field, &[value], 1, byte);
    f.ret(field);
    assert!(verify_graph(&mut f.graph).is_err());
}

#[test]
fn char_and_enum_validation_is_deferred_to_typed_read() {
    let mut types = RecordedTypes::new();
    let character = types.intern(TypeShape::Char, Some(SizeAlign { size: 4, align: 4 }));
    let mu_char = wrapper(&mut types, character, true);
    let byte = scalar(&mut types, IntegerWidth::W8);
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "TwoTags".into(),
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    types.record_aggregate(
        enumeration,
        AggregateLayout {
            fields: vec![],
            variants: vec![vec![], vec![]],
        },
    );
    types.record_semantic_enum(enumeration, vec![3, 7]);
    types.enums.insert(
        enumeration,
        kfrc_body::EnumLayout {
            tag: Some(FieldLayout {
                ty: byte,
                offset: 0,
            }),
            encoding: kfrc_body::TagEncoding::Direct {
                discriminants: vec![3, 7],
            },
        },
    );
    let mu_enum = wrapper(&mut types, enumeration, true);
    for (mu, inner, bytes) in [
        (
            mu_char,
            character,
            vec![Some(0x00), Some(0xd8), Some(0), Some(0)],
        ),
        (mu_enum, enumeration, vec![Some(9)]),
    ] {
        let value = raw(&bytes);
        let mut f = Fixture::new(types.clone(), &[mu]);
        f.ret(f.args[0]);
        assert_eq!(
            f.run(&[value.clone()]),
            Ok(SemanticOutcome::Return(value.clone()))
        );
        let mut f = Fixture::new(types.clone(), &[mu]);
        let typed = f.transmute(f.args[0], inner);
        f.ret(typed);
        assert!(matches!(
            f.run(&[value]),
            Err(InterpretError::UndefinedBehaviour(_))
        ));
    }
}

#[test]
fn initialized_bool_roundtrip_preserves_values() {
    let mut types = RecordedTypes::new();
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mu = wrapper(&mut types, boolean, true);
    let mut f = Fixture::new(types, &[boolean]);
    let wrapped = f.transmute(f.args[0], mu);
    let restored = f.transmute(wrapped, boolean);
    f.ret(restored);
    for value in [0, 1] {
        assert_eq!(
            f.run(&[Value::Scalar(value)]),
            Ok(SemanticOutcome::Return(Value::Scalar(value)))
        );
    }
}
