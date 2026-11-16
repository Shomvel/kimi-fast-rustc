use super::*;
use kfrc_body::TypeOracle;

fn array(f: &mut Fixture, element: TyId, length: u64) -> TyId {
    let item = f.graph.oracle.size_align(element).unwrap();
    f.graph.oracle.intern(
        TypeShape::Array { element, length },
        Some(SizeAlign {
            size: item.size * length,
            align: item.align,
        }),
    )
}

fn index(f: &mut Fixture, base: NodeId, index: u64, element: TyId) -> NodeId {
    let offset = f.graph.add_constant(u128::from(index), f.word);
    let pointer = f.pointer(element);
    f.graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[base, offset],
        0,
        ValueType::Rust(pointer),
    )
}

#[test]
fn fixed_array_index_reads_old_snapshot_after_a_field_write() {
    let mut f = Fixture::new();
    let word = f.word;
    let ty = array(&mut f, word, 2);
    let slot = f.slot(0, ty);
    let seven = f.graph.add_constant(7, word);
    let values = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[f.limit, seven],
        0,
        ValueType::Rust(ty),
    );
    f.store(slot, values);
    let address = index(&mut f, slot, 1, word);
    let old = f.load(address, word);
    f.store(address, f.limit);
    let snapshot = f.load(slot, ty);
    let current = f.graph.add_node(
        Opcode::Field,
        NodeId::NONE,
        &[snapshot],
        1,
        ValueType::Rust(word),
    );
    let result = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[old, current],
        0,
        ValueType::Rust(word),
    );
    f.ret(result);
    let stats = assert_equivalent(&mut f, &[(false, 0), (true, 17), (false, u64::MAX)]);
    assert_eq!(
        (
            stats.slots_promoted,
            stats.loads_removed,
            stats.stores_removed
        ),
        (1, 2, 2)
    );
}

#[test]
fn a_fixed_index_initializes_only_its_array_element() {
    for selected in [0, 1] {
        let mut f = Fixture::new();
        let word = f.word;
        let ty = array(&mut f, word, 2);
        let slot = f.slot(0, ty);
        let written = index(&mut f, slot, 1, word);
        f.store(written, f.limit);
        let read = index(&mut f, slot, selected, word);
        let result = f.load(read, word);
        f.ret(result);
        let stats = assert_equivalent(&mut f, &[(false, 17)]);
        assert_eq!(stats.slots_promoted, 1);
        let outcome = execute(&f.graph, false, 17);
        if selected == 1 {
            assert_eq!(outcome.unwrap(), SemanticOutcome::Return(Value::Scalar(17)));
        } else {
            assert!(matches!(outcome, Err(InterpretError::PoisonObserved(_))));
        }
    }
}

#[test]
fn nested_array_indices_and_field_addresses_share_one_logical_path() {
    let mut f = Fixture::new();
    let word = f.word;
    let inner = array(&mut f, word, 2);
    let outer = array(&mut f, inner, 2);
    let slot = f.slot(0, outer);
    let row = index(&mut f, slot, 1, inner);
    let cell = index(&mut f, row, 0, word);
    f.store(cell, f.limit);
    let row = f.field(slot, 1, inner);
    let cell = f.field(row, 0, word);
    let value = f.load(cell, word);
    f.ret(value);
    let stats = assert_equivalent(&mut f, &[(false, 0), (true, u64::MAX)]);
    assert_eq!(
        (
            stats.slots_promoted,
            stats.loads_removed,
            stats.stores_removed
        ),
        (1, 1, 1)
    );
}

#[test]
fn invalid_or_pointer_arithmetic_indices_retain_the_access() {
    for (offset, whole_array) in [(2, false), (u64::MAX, false), (1, true)] {
        let mut f = Fixture::new();
        let word = f.word;
        let ty = array(&mut f, word, 2);
        let slot = f.slot(0, ty);
        let element = if whole_array { ty } else { word };
        let address = index(&mut f, slot, offset, element);
        let value = f.load(address, element);
        let result = if whole_array {
            f.graph.add_node(
                Opcode::Field,
                NodeId::NONE,
                &[value],
                0,
                ValueType::Rust(word),
            )
        } else {
            value
        };
        f.ret(result);
        let stats = assert_equivalent(&mut f, &[(false, 17)]);
        assert_eq!(stats.slots_promoted, 0);
    }
}

#[test]
fn source_width_and_sign_boundaries_retain_ambiguous_indices() {
    for (width, signed, raw) in [
        (IntegerWidth::W8, false, 1u128 << 8),
        (IntegerWidth::W16, false, 1 << 16),
        (IntegerWidth::W32, false, 1 << 32),
        (IntegerWidth::W64, false, 1 << 64),
        (IntegerWidth::W8, true, (1 << 8) - 1),
        (IntegerWidth::W16, true, (1 << 16) - 1),
        (IntegerWidth::W32, true, (1 << 32) - 1),
        (IntegerWidth::W64, true, (1 << 64) - 1),
    ] {
        let mut f = Fixture::new();
        let word = f.word;
        let array = array(&mut f, word, 2);
        let slot = f.slot(0, array);
        let bytes = u64::from(width.bits(64)) / 8;
        let index_ty = f.graph.oracle.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign {
                size: bytes,
                align: bytes,
            }),
        );
        let index = f.graph.add_constant(raw, index_ty);
        let pointer = f.pointer(word);
        let address = f.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[slot, index],
            0,
            ValueType::Rust(pointer),
        );
        f.store(address, f.limit);
        let result = f.load(address, word);
        f.ret(result);
        assert_eq!(assert_equivalent(&mut f, &[(false, 17)]).slots_promoted, 0);
    }
}

#[test]
fn an_index_made_constant_at_a_seam_retries_the_owning_slot() {
    let mut f = Fixture::new();
    let word = f.word;
    let ty = array(&mut f, word, 2);
    let slot = f.slot(0, ty);
    let pointer = f.pointer(word);
    let address = f.graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[slot, f.limit],
        0,
        ValueType::Rust(pointer),
    );
    let answer = f.graph.add_constant(42, word);
    f.store(address, answer);
    let value = f.load(address, word);
    f.ret(value);
    verify_graph(&mut f.graph).unwrap();
    let one = f.graph.add_constant(1, word);
    f.graph.replace(f.limit, one);
    f.graph.resolve_edges();
    let stats = crate::optimize_specialized_seams(&mut f.graph, &[address], &[]).unwrap();
    assert_eq!(stats.slots_promoted, 1);
    assert!(!f.graph.ids().any(|node| matches!(
        f.graph.op(node),
        Opcode::LoadValue | Opcode::StoreValue | Opcode::LocalSlot
    )));
    assert_eq!(
        execute(&f.graph, false, 99).unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
}
