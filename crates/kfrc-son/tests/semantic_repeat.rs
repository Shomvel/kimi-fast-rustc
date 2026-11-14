use kfrc_body::fx_hash::HashMap;
use kfrc_body::{
    FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape,
};
use kfrc_son::{
    FunctionGraph, InterpretError, Interpreter, NodeId, Opcode, SemanticOutcome, Value, ValueType,
    verify_graph,
};
use std::sync::Arc;

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    array: TyId,
    control: NodeId,
    effect: NodeId,
}
impl Fixture {
    fn new(length: u64) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let array = types.intern(
            TypeShape::Array {
                element: word,
                length,
            },
            Some(SizeAlign {
                size: length * 8,
                align: 8,
            }),
        );
        let mut graph = FunctionGraph::new(
            "repeat",
            FunctionAbi {
                args: vec![],
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
            array,
            control,
            effect,
        }
    }
    fn node(&mut self, op: Opcode, inputs: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, inputs, payload, ValueType::Rust(ty))
    }
    fn repeat(&mut self, element: NodeId) -> NodeId {
        self.node(Opcode::Repeat, &[element], 0, self.array)
    }
    fn field(&mut self, value: NodeId, field: u32) -> NodeId {
        self.node(Opcode::Field, &[value], field, self.word)
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
    fn run(&mut self) -> Result<SemanticOutcome, InterpretError> {
        verify_graph(&mut self.graph).unwrap();
        Interpreter::new(&self.graph).run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
    }
}

#[test]
fn repeated_values_keep_u64_length_and_constant_storage() {
    for length in [1, 4096, 1u64 << 32] {
        let mut f = Fixture::new(length);
        let seven = f.graph.add_constant(7, f.word);
        let repeat = f.repeat(seven);
        f.ret(repeat);
        let SemanticOutcome::Return(Value::Array {
            element,
            length: actual,
            overrides,
        }) = f.run().unwrap()
        else {
            panic!("repeat expanded its value");
        };
        assert_eq!(actual, length);
        assert_eq!(*element, Value::Scalar(7));
        assert!(overrides.is_empty());
        assert_eq!(f.graph.len(), 6);
        assert_eq!(
            f.graph.oracle.semantic().field_count(f.array, None),
            Some(length)
        );
    }
}

#[test]
fn maximum_logical_field_index_does_not_expand_the_array() {
    let mut f = Fixture::new(1u64 << 32);
    let nine = f.graph.add_constant(9, f.word);
    let repeat = f.repeat(nine);
    let last = f.field(repeat, u32::MAX);
    f.ret(last);
    assert_eq!(f.run(), Ok(SemanticOutcome::Return(Value::Scalar(9))));
}

#[test]
fn empty_repeat_ignores_poison_element_but_preserves_fixed_traps() {
    let mut f = Fixture::new(0);
    let poison = f.node(Opcode::Undefined, &[], 0, f.word);
    let empty = f.repeat(poison);
    f.ret(empty);
    let outcome = f.run().unwrap();
    assert_eq!(outcome, SemanticOutcome::Return(Value::Aggregate(vec![])));
    let SemanticOutcome::Return(value) = outcome else {
        unreachable!()
    };
    assert!(!value.contains_poison());

    let mut f = Fixture::new(0);
    let one = f.graph.add_constant(1, f.word);
    let zero = f.graph.add_constant(0, f.word);
    let trap = f.graph.add_node(
        Opcode::DivUnsigned,
        f.control,
        &[one, zero],
        0,
        ValueType::Rust(f.word),
    );
    let empty = f.repeat(trap);
    f.ret(empty);
    assert!(matches!(
        f.run(),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    kfrc_son::optimize_specialized_graph(&mut f.graph).unwrap();
    assert!(matches!(
        f.run(),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
}

#[test]
fn sparse_updates_keep_snapshot_and_uninitialized_leaves_independent() {
    let mut f = Fixture::new(4096);
    let poison = f.node(Opcode::Undefined, &[], 0, f.array);
    let seven = f.graph.add_constant(7, f.word);
    let modified = f.node(Opcode::InsertField, &[poison, seven], 4095, f.array);
    let value = f.field(modified, 4095);
    f.ret(value);
    assert_eq!(f.run(), Ok(SemanticOutcome::Return(Value::Scalar(7))));

    let mut f = Fixture::new(4096);
    let seven = f.graph.add_constant(7, f.word);
    let original = f.repeat(seven);
    let nine = f.graph.add_constant(9, f.word);
    let modified = f.node(Opcode::InsertField, &[original, nine], 4095, f.array);
    let pair = f
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![f.array, f.array]), None);
    let both = f.node(Opcode::Aggregate, &[original, modified], 0, pair);
    f.ret(both);
    let SemanticOutcome::Return(Value::Aggregate(values)) = f.run().unwrap() else {
        panic!("tuple result");
    };
    let [
        Value::Array {
            element: left,
            overrides: left_overrides,
            ..
        },
        Value::Array {
            element: right,
            overrides: right_overrides,
            ..
        },
    ] = values.as_slice()
    else {
        panic!("compact snapshots");
    };
    assert!(Arc::ptr_eq(left, right));
    assert!(left_overrides.is_empty());
    assert_eq!(right_overrides.len(), 1);
    assert_eq!(values[0].array_element(4095), Some(&Value::Scalar(7)));
    assert_eq!(values[1].array_element(4095), Some(&Value::Scalar(9)));
}

#[test]
fn overriding_every_poison_element_makes_only_those_elements_initialized() {
    let poison = Value::array(Value::Poison, 2);
    let full = Value::Array {
        element: Arc::new(Value::Poison),
        length: 2,
        overrides: Arc::new(HashMap::from_iter([
            (0, Value::Scalar(7)),
            (1, Value::Scalar(7)),
        ])),
    };
    assert!(poison.contains_poison());
    assert!(!full.contains_poison());
    assert_eq!(full, Value::array(Value::Scalar(7), 2));
    assert_eq!(
        full,
        Value::Aggregate(vec![Value::Scalar(7), Value::Scalar(7)])
    );
    let partial = Value::Array {
        element: Arc::new(Value::Poison),
        length: 3,
        overrides: Arc::new(HashMap::from_iter([
            (0, Value::Scalar(7)),
            (1, Value::Scalar(7)),
        ])),
    };
    assert!(partial.contains_poison());
    assert_ne!(partial, Value::array(Value::Scalar(7), 3));
}

#[test]
fn typed_memory_roundtrip_keeps_repeat_snapshot_compact() {
    let mut f = Fixture::new(4096);
    let pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: f.array,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slot = f.node(Opcode::LocalSlot, &[], 0, pointer);
    let seven = f.graph.add_constant(7, f.word);
    let original = f.repeat(seven);
    f.fixed(Opcode::StoreValue, &[slot, original], None);
    let snapshot = f.fixed(Opcode::LoadValue, &[slot], Some(f.array)).unwrap();
    let nine = f.graph.add_constant(9, f.word);
    let updated = f.node(Opcode::InsertField, &[original, nine], 4095, f.array);
    f.fixed(Opcode::StoreValue, &[slot, updated], None);
    f.ret(snapshot);
    let SemanticOutcome::Return(value @ Value::Array { .. }) = f.run().unwrap() else {
        panic!("memory snapshot expanded");
    };
    assert_eq!(value.array_element(4095), Some(&Value::Scalar(7)));
    let Value::Array { overrides, .. } = value else {
        unreachable!()
    };
    assert!(overrides.is_empty());
}

#[test]
fn repeat_verifier_rejects_count_payload_element_type_and_bad_projection() {
    let mut f = Fixture::new(4);
    let seven = f.graph.add_constant(7, f.word);
    let repeat = f.repeat(seven);
    f.ret(repeat);
    verify_graph(&mut f.graph).unwrap();
    f.graph.set_payload(repeat, 4);
    assert!(verify_graph(&mut f.graph).is_err());
    f.graph.set_payload(repeat, 0);
    f.graph.set_value_type(repeat, ValueType::Rust(f.word));
    assert!(verify_graph(&mut f.graph).is_err());

    let mut f = Fixture::new(4);
    let seven = f.graph.add_constant(7, f.word);
    let repeat = f.repeat(seven);
    let past_end = f.field(repeat, 4);
    f.ret(past_end);
    assert!(verify_graph(&mut f.graph).is_err());
}
