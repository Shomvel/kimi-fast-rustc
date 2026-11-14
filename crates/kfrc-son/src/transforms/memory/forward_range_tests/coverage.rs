use super::*;

#[test]
fn whole_object_borrow_preserves_each_field_and_call_time_memory() {
    for field in [0, 1] {
        for source_load in [false, true] {
            let mut f = Fixture::new();
            let address = f.field(f.root, f.pair, field, f.word);
            let value = f.graph.add_constant(0x1234_5678_9abc_def0, f.word);
            let source = if source_load {
                f.load(address, f.word)
            } else {
                f.store(address, value);
                value
            };
            let borrowed = f.reference(f.root, f.pair, Mutability::Not);
            let call = f.call(borrowed, frozen(16));
            let loaded = f.load(address, f.word);
            let sum = f.graph.add_node(
                Opcode::Add,
                NodeId::NONE,
                &[source, loaded],
                0,
                ValueType::Rust(f.word),
            );
            f.ret(sum);
            assert!(f.graph.dump().contains(&format!(
                "call_permissions {call} callee=observe p0:Frozen(16)"
            )));
            assert_eq!(f.check(1, &[Run::default()]).call_permissions_used, 1);
            let result = execute(&f.graph, &Run::default());
            let expected: u64 = if source_load {
                0x5a5a_5a5a_5a5a_5a5a
            } else {
                0x1234_5678_9abc_def0
            };
            assert_eq!(
                result.result,
                Ok(SemanticOutcome::Return(Value::Scalar(
                    expected.wrapping_mul(2)
                )))
            );
            assert_eq!(result.calls.len(), 1);
            assert_eq!(result.calls[0], result.bytes);
        }
    }
}

#[test]
fn nested_fields_work_with_an_offset_borrow_or_the_whole_object() {
    for whole_object in [false, true] {
        let mut f = Fixture::new();
        let inner = f.field(f.root, f.nested, 1, f.pair);
        let address = f.field(inner, f.pair, 1, f.word);
        let value = f.graph.add_constant(93, f.word);
        f.store(address, value);
        let borrowed = if whole_object {
            f.reference(f.root, f.nested, Mutability::Not)
        } else {
            f.reference(inner, f.pair, Mutability::Not)
        };
        f.call(borrowed, frozen(if whole_object { 24 } else { 16 }));
        let loaded = f.load(address, f.word);
        f.ret(loaded);
        f.check(1, &[Run::default()]);
        assert_eq!(
            execute(&f.graph, &Run::default()).result,
            Ok(SemanticOutcome::Return(Value::Scalar(93)))
        );
    }
}

#[test]
fn a_typed_product_snapshot_is_preserved_as_a_whole_value() {
    let mut f = Fixture::new();
    let address = f.cast(f.root, f.pair);
    let first = f.graph.add_constant(41, f.word);
    let second = f.graph.add_constant(97, f.word);
    let product = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[first, second],
        0,
        ValueType::Rust(f.pair),
    );
    f.store(address, product);
    let borrowed = f.reference(f.root, f.nested, Mutability::Not);
    f.call(borrowed, frozen(24));
    let loaded = f.load(address, f.pair);
    let selected = f.graph.add_node(
        Opcode::Field,
        NodeId::NONE,
        &[loaded],
        1,
        ValueType::Rust(f.word),
    );
    f.ret(selected);
    f.check(1, &[Run::default()]);
    assert_eq!(
        execute(&f.graph, &Run::default()).result,
        Ok(SemanticOutcome::Return(Value::Scalar(97)))
    );
}

#[test]
fn a_sized_array_borrow_covers_a_canonical_constant_element() {
    let mut f = Fixture::new();
    let array = f.graph.oracle.intern(
        TypeShape::Array {
            element: f.word,
            length: 4,
        },
        Some(SizeAlign { size: 32, align: 8 }),
    );
    let index = f.graph.add_constant(2, f.word);
    let address = f.index(f.root, f.word, index);
    let value = f.graph.add_constant(83, f.word);
    f.store(address, value);
    let borrowed = f.reference(f.root, array, Mutability::Not);
    f.call(borrowed, frozen(32));
    let loaded = f.load(address, f.word);
    f.ret(loaded);
    f.check(1, &[Run::default()]);
    assert_eq!(
        execute(&f.graph, &Run::default()).result,
        Ok(SemanticOutcome::Return(Value::Scalar(83)))
    );
}
