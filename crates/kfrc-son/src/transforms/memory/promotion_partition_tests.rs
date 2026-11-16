use super::*;

#[path = "promotion_partition_tests/value_safety.rs"]
mod value_safety;

fn narrow(f: &mut Fixture, address: NodeId, width: IntegerWidth) -> NodeId {
    let ty = f.graph.oracle.intern(
        TypeShape::Int {
            width,
            signed: false,
        },
        Some(SizeAlign {
            size: u64::from(width.bits(64)) / 8,
            align: u64::from(width.bits(64)) / 8,
        }),
    );
    let pointer = f.pointer(ty);
    let address = f.graph.add_node(
        Opcode::PointerCast,
        NodeId::NONE,
        &[address],
        0,
        ValueType::Rust(pointer),
    );
    let value = f.load(address, ty);
    f.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(f.word),
    )
}

fn initialize_pair(f: &mut Fixture, slot: NodeId) {
    let seven = f.graph.add_constant(7, f.word);
    let value = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[f.limit, seven],
        0,
        ValueType::Rust(f.pair),
    );
    f.store(slot, value);
}

fn add(f: &mut Fixture, a: NodeId, b: NodeId) -> NodeId {
    f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[a, b],
        0,
        ValueType::Rust(f.word),
    )
}

fn samples() -> Vec<(bool, u64)> {
    let mut state = 0x8047_13db_9546_af21u64;
    (0..96)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (i % 2 != 0, state)
        })
        .chain([(false, 0), (true, 1), (false, u64::MAX)])
        .collect()
}

#[test]
fn independent_field_promotes_while_retyped_sibling_keeps_storage() {
    for width in [IntegerWidth::W8, IntegerWidth::W16, IntegerWidth::W32] {
        let mut f = Fixture::new();
        let slot = f.slot(0, f.pair);
        initialize_pair(&mut f, slot);
        let first = f.field(slot, 0, f.word);
        let second = f.field(slot, 1, f.word);
        let old = f.load(first, f.word);
        let seven = f.graph.add_constant(7, f.word);
        let next = add(&mut f, old, seven);
        f.store(first, next);
        let retained = narrow(&mut f, second, width);
        let current = f.load(first, f.word);
        let sum = add(&mut f, old, current);
        let result = add(&mut f, sum, retained);
        f.ret(result);
        let stats = assert_equivalent(&mut f, &samples());
        assert_eq!(
            (
                stats.slots_promoted,
                stats.loads_removed,
                stats.stores_removed
            ),
            (1, 2, 1)
        );
        assert_eq!(
            f.graph
                .ids()
                .filter(|&n| f.graph.op(n) == Opcode::StoreValue)
                .count(),
            1,
            "the whole initializer still materializes the retained sibling"
        );
    }
}

#[test]
fn partitions_follow_observed_indices_across_array_lengths() {
    for length in [2, 3, 9] {
        let mut f = Fixture::new();
        let word = f.word;
        let array = f.graph.oracle.intern(
            TypeShape::Array {
                element: word,
                length,
            },
            Some(SizeAlign {
                size: 8 * length,
                align: 8,
            }),
        );
        let slot = f.slot(0, array);
        let inputs = vec![f.limit; length as usize];
        let initial = f.graph.add_node(
            Opcode::Aggregate,
            NodeId::NONE,
            &inputs,
            0,
            ValueType::Rust(array),
        );
        f.store(slot, initial);
        let selected = f.field(slot, (length / 2 - 1) as u32, word);
        let other = f.field(slot, (length - 1) as u32, word);
        let old = f.load(selected, word);
        let seven = f.graph.add_constant(7, word);
        f.store(selected, seven);
        let current = f.load(selected, word);
        let retained = narrow(&mut f, other, IntegerWidth::W32);
        let sum = add(&mut f, old, current);
        let result = add(&mut f, sum, retained);
        f.ret(result);
        let stats = assert_equivalent(&mut f, &samples());
        assert_eq!((stats.loads_removed, stats.stores_removed), (2, 1));
    }
}

#[test]
fn partial_field_values_join_after_branch_updates() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.pair);
    initialize_pair(&mut f, slot);
    let first = f.field(slot, 0, f.word);
    let second = f.field(slot, 1, f.word);
    let retained = narrow(&mut f, second, IntegerWidth::W32);
    let effect = f.effect;
    let (yes, no) = f.branch(f.condition);
    f.control = yes;
    let seven = f.graph.add_constant(7, f.word);
    f.store(first, seven);
    let yes_effect = f.effect;
    let yes_jump = f.jump();
    f.control = no;
    f.effect = effect;
    f.store(first, f.limit);
    let no_effect = f.effect;
    let no_jump = f.jump();
    let region = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[yes_jump, no_jump],
        0,
        ValueType::Control,
    );
    f.control = region;
    f.effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, yes_effect, no_effect],
        0,
        ValueType::Effect,
    );
    let value = f.load(first, f.word);
    let result = add(&mut f, value, retained);
    f.ret(result);
    let stats = assert_equivalent(&mut f, &samples());
    assert_eq!((stats.loads_removed, stats.stores_removed), (1, 2));
    assert!(stats.value_phis > 0);
}

#[test]
fn enclosing_snapshot_keeps_writes_visible_in_memory() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.pair);
    initialize_pair(&mut f, slot);
    let first = f.field(slot, 0, f.word);
    let second = f.field(slot, 1, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(first, seven);
    let retained = narrow(&mut f, second, IntegerWidth::W32);
    let snapshot = f.load(slot, f.pair);
    let value = f.graph.add_node(
        Opcode::Field,
        NodeId::NONE,
        &[snapshot],
        0,
        ValueType::Rust(f.word),
    );
    let result = add(&mut f, value, retained);
    f.ret(result);
    let stats = assert_equivalent(&mut f, &samples());
    assert_eq!(stats.stores_removed, 0);
}

#[test]
fn pointer_arithmetic_may_access_a_different_logical_field() {
    for constant in [false, true] {
        let mut f = Fixture::new();
        let slot = f.slot(0, f.pair);
        initialize_pair(&mut f, slot);
        let first = f.field(slot, 0, f.word);
        let second = f.field(slot, 1, f.word);
        let thirteen = f.graph.add_constant(13, f.word);
        f.store(second, thirteen);
        let offset = if constant {
            f.graph.add_constant(1, f.word)
        } else {
            f.limit
        };
        let pointer = f.pointer(f.word);
        let address = f.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[first, offset],
            0,
            ValueType::Rust(pointer),
        );
        let observed = f.load(address, f.word);
        let value = f.load(second, f.word);
        let result = add(&mut f, observed, value);
        f.ret(result);
        let stats = assert_equivalent(&mut f, &[(false, 0), (true, 1)]);
        assert_eq!(stats.stores_removed, 0);
    }
}

#[test]
fn storage_lifetime_is_retained_for_the_unpromoted_region() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.pair);
    f.fixed(Opcode::SlotDead, &[slot], 0, None);
    f.fixed(Opcode::SlotLive, &[slot], 0, None);
    initialize_pair(&mut f, slot);
    let first = f.field(slot, 0, f.word);
    let second = f.field(slot, 1, f.word);
    let old = f.load(first, f.word);
    f.store(first, f.limit);
    let retained = narrow(&mut f, second, IntegerWidth::W32);
    let result = add(&mut f, old, retained);
    f.ret(result);
    let stats = assert_equivalent(&mut f, &samples());
    assert_eq!((stats.loads_removed, stats.stores_removed), (1, 1));
    assert_eq!(stats.lifetime_ops_removed, 0);
}

#[test]
fn copy_extent_includes_every_element_that_can_observe_a_partition() {
    for count in [1, 2] {
        let mut f = Fixture::new();
        let word = f.word;
        let array = f.graph.oracle.intern(
            TypeShape::Array {
                element: word,
                length: 3,
            },
            Some(SizeAlign { size: 24, align: 8 }),
        );
        let source = f.slot(0, array);
        let destination = f.slot(1, array);
        let initial = f.graph.add_node(
            Opcode::Aggregate,
            NodeId::NONE,
            &[f.limit; 3],
            0,
            ValueType::Rust(array),
        );
        f.store(source, initial);
        let copied = f.field(source, 1, word);
        let candidate = f.field(source, 2, word);
        let target = f.field(destination, 0, word);
        let seven = f.graph.add_constant(7, word);
        f.store(candidate, seven);
        let count_node = f.graph.add_constant(count, word);
        f.fixed(
            Opcode::CopyNonOverlapping,
            &[copied, target, count_node],
            0,
            None,
        );
        let observed = f.field(destination, count as u32 - 1, word);
        let retained = f.load(observed, word);
        let value = f.load(candidate, word);
        let result = add(&mut f, value, retained);
        f.ret(result);
        let stats = assert_equivalent(&mut f, &samples());
        let expected = usize::from(count == 1);
        assert_eq!(
            (stats.loads_removed, stats.stores_removed),
            (expected, expected)
        );
    }
}

#[test]
fn multiple_disjoint_partitions_share_a_retained_whole_initializer() {
    let mut f = Fixture::new();
    let word = f.word;
    let array = f.graph.oracle.intern(
        TypeShape::Array {
            element: word,
            length: 3,
        },
        Some(SizeAlign { size: 24, align: 8 }),
    );
    let slot = f.slot(0, array);
    let initial = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[f.limit; 3],
        0,
        ValueType::Rust(array),
    );
    f.store(slot, initial);
    let left = f.field(slot, 0, word);
    let middle = f.field(slot, 1, word);
    let right = f.field(slot, 2, word);
    let old = f.load(left, word);
    let retained = narrow(&mut f, middle, IntegerWidth::W32);
    let next = add(&mut f, old, retained);
    f.store(left, next);
    f.store(right, old);
    let left_value = f.load(left, word);
    let right_value = f.load(right, word);
    let result = add(&mut f, left_value, right_value);
    f.ret(result);
    let stats = assert_equivalent(&mut f, &samples());
    assert_eq!((stats.slots_promoted, stats.partitions_promoted), (1, 2));
    assert_eq!((stats.loads_removed, stats.stores_removed), (3, 2));
}

#[test]
fn partition_ssa_preserves_loop_backedges_and_zero_iterations() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.pair);
    initialize_pair(&mut f, slot);
    let first = f.field(slot, 0, f.word);
    let second = f.field(slot, 1, f.word);
    let retained = narrow(&mut f, second, IntegerWidth::W32);
    let zero = f.graph.add_constant(0, f.word);
    f.store(first, zero);
    let entry_effect = f.effect;
    let entry = f.jump();
    let region = f
        .graph
        .add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    let effect_phi = f
        .graph
        .add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Effect);
    f.control = region;
    f.effect = effect_phi;
    let value = f.load(first, f.word);
    let iteration_effect = f.effect;
    let condition = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[value, f.limit],
        crate::ComparePredicate::UnsignedLess as u32,
        ValueType::Rust(f.boolean),
    );
    let (body, exit) = f.branch(condition);
    f.control = body;
    let one = f.graph.add_constant(1, f.word);
    let next = add(&mut f, value, one);
    f.store(first, next);
    let back_effect = f.effect;
    let back = f.jump();
    f.graph.set_inputs(region, &[entry, back]);
    f.graph
        .set_inputs(effect_phi, &[region, entry_effect, back_effect]);
    f.control = exit;
    f.effect = iteration_effect;
    let result = add(&mut f, value, retained);
    f.ret(result);
    let stats = assert_equivalent(&mut f, &[(false, 0), (true, 1), (false, 9)]);
    assert_eq!((stats.loads_removed, stats.stores_removed), (1, 2));
    assert_eq!(stats.partitions_promoted, 1);
    assert_eq!(stats.value_phis, 1);
}

#[test]
fn a_new_storage_lifetime_does_not_reuse_the_old_partition_value() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.pair);
    initialize_pair(&mut f, slot);
    let first = f.field(slot, 0, f.word);
    let second = f.field(slot, 1, f.word);
    f.fixed(Opcode::SlotDead, &[slot], 0, None);
    f.fixed(Opcode::SlotLive, &[slot], 0, None);
    f.store(second, f.limit);
    let value = f.load(first, f.word);
    let retained = narrow(&mut f, second, IntegerWidth::W32);
    let result = add(&mut f, value, retained);
    f.ret(result);
    let stats = assert_equivalent(&mut f, &[(false, 17)]);
    assert_eq!(stats.partitions_promoted, 1);
    assert!(matches!(
        execute(&f.graph, false, 17),
        Err(InterpretError::PoisonObserved(_))
    ));
    assert_eq!(stats.lifetime_ops_removed, 0);
}

#[test]
fn full_and_seeded_production_pipelines_consume_partial_promotion() {
    for seeded in [false, true] {
        let mut f = Fixture::new();
        f.graph.level = GraphLevel::Specialized;
        let slot = f.slot(0, f.pair);
        initialize_pair(&mut f, slot);
        let first = f.field(slot, 0, f.word);
        let second = f.field(slot, 1, f.word);
        f.store(first, f.limit);
        let retained = narrow(&mut f, second, IntegerWidth::W32);
        let value = f.load(first, f.word);
        let result = add(&mut f, value, retained);
        f.ret(result);
        verify_graph(&mut f.graph).unwrap();
        let inputs = samples();
        let before: Vec<_> = inputs
            .iter()
            .map(|&(condition, limit)| execute(&f.graph, condition, limit))
            .collect();
        let stats = if seeded {
            crate::optimize_specialized_seams(&mut f.graph, &[first], &[slot])
        } else {
            crate::optimize_specialized_graph(&mut f.graph)
        }
        .unwrap();
        verify_graph(&mut f.graph).unwrap();
        for ((condition, limit), expected) in inputs.into_iter().zip(before) {
            assert_eq!(execute(&f.graph, condition, limit), expected);
        }
        assert_eq!(stats.partitions_promoted, 1);
    }
}
