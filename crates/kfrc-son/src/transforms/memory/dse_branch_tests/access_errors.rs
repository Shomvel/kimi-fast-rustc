use super::*;

/// Build either a straight suffix or a two-arm suffix whose other arm covers
/// immediately. The tested arm performs a disjoint store before its overwrite.
fn suffix_with_disjoint_store(
    f: &mut Fixture,
    target: NodeId,
    other: NodeId,
    other_value: NodeId,
    replacement: NodeId,
) {
    if let Some(&condition) = f.conditions.first() {
        let states = f.branch(condition);
        f.enter(states[0]);
        f.store(target, replacement);
        f.load_return(target);
        f.enter(states[1]);
    }
    f.store(other, other_value);
    f.store(target, replacement);
    f.load_return(target);
}

fn failing_input(branched: bool) -> Vec<Value> {
    vec![Value::Scalar(0); usize::from(branched)]
}

fn assert_access_error(execution: &Execution, detail: &str) {
    assert!(
        matches!(&execution.result, Err(InterpretError::UndefinedBehaviour(message)) if message == detail),
        "{execution:?}"
    );
}

#[test]
fn out_of_bounds_field_and_index_stores_stop_straight_and_branched_dse() {
    for branched in [false, true] {
        for field in [false, true] {
            let mut f = Fixture::new(usize::from(branched), true);
            let left = f.field(f.address, 0);
            let one = f.graph.add_constant(1, f.word);
            let two = f.graph.add_constant(2, f.word);
            let other = if field {
                // This is a legal field projection of a larger retyped
                // pointer, but the actual protected allocation is 16 bytes.
                let array = f.graph.oracle.intern(
                    TypeShape::Array {
                        element: f.word,
                        length: 4,
                    },
                    Some(SizeAlign { size: 32, align: 8 }),
                );
                let retyped = f.cast(f.address, array);
                f.field(retyped, 3)
            } else {
                let pointer = f.graph.rust_type(left).unwrap();
                let index = f.graph.add_constant(3, f.word);
                f.graph.add_node(
                    Opcode::IndexAddr,
                    NodeId::NONE,
                    &[left, index],
                    0,
                    ValueType::Rust(pointer),
                )
            };
            let old = f.store(left, one);
            suffix_with_disjoint_store(&mut f, left, other, two, two);
            let original = f.graph.clone();
            assert_eq!(f.check(old, false).stores_removed, 0);
            let input = failing_input(branched);
            let before = execute(&original, &input, false);
            assert_access_error(&before, "memory access outside an allocation");
            assert_eq!(before.bytes[..8], 1u64.to_le_bytes().map(Some));
            assert_eq!(execute(&f.graph, &input, false), before);
        }
    }
}

#[test]
fn an_in_bounds_but_unaligned_disjoint_store_preserves_the_old_write_on_error() {
    for branched in [false, true] {
        let mut f = Fixture::new(usize::from(branched), true);
        let left = f.field(f.address, 0);
        let byte = f.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let lane = f.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: false,
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        let bytes = f.cast(f.address, byte);
        let pointer = f.graph.rust_type(bytes).unwrap();
        let nine = f.graph.add_constant(9, f.word);
        let unaligned = f.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[bytes, nine],
            0,
            ValueType::Rust(pointer),
        );
        let other = f.cast(unaligned, lane);
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let lane_value = f.graph.add_constant(3, lane);
        let old = f.store(left, one);
        // [9,13) lies inside [0,16), disjoint from [0,8), but fails align 4.
        suffix_with_disjoint_store(&mut f, left, other, lane_value, two);
        let original = f.graph.clone();
        assert_eq!(f.check(old, false).stores_removed, 0);
        let input = failing_input(branched);
        let before = execute(&original, &input, false);
        assert_access_error(&before, "null or misaligned memory access");
        assert_eq!(before.bytes[..8], 1u64.to_le_bytes().map(Some));
        assert_eq!(execute(&f.graph, &input, false), before);
    }
}

fn another_local(f: &mut Fixture) -> NodeId {
    let pointer = f.pointer(f.pair);
    f.graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        1,
        ValueType::Rust(pointer),
    )
}

#[test]
fn a_disjoint_dead_local_is_a_barrier_for_both_control_shapes() {
    for branched in [false, true] {
        for local_target in [false, true] {
            let mut f = Fixture::new(usize::from(branched), true);
            let dead = f.local();
            let other = f.field(dead, 0);
            let target_root = if local_target {
                another_local(&mut f)
            } else {
                f.address
            };
            let target = f.field(target_root, 0);
            let one = f.graph.add_constant(1, f.word);
            let two = f.graph.add_constant(2, f.word);
            f.fixed(Opcode::SlotDead, &[dead], None);
            let old = f.store(target, one);
            suffix_with_disjoint_store(&mut f, target, other, two, two);
            let original = f.graph.clone();
            assert_eq!(f.check(old, false).stores_removed, 0);
            let input = failing_input(branched);
            let before = execute(&original, &input, false);
            assert_access_error(&before, "memory access outside storage lifetime");
            if !local_target {
                assert_eq!(before.bytes[..8], 1u64.to_le_bytes().map(Some));
            }
            assert_eq!(execute(&f.graph, &input, false), before);
        }
    }
}

#[test]
fn unchecked_raw_disjoint_ranges_keep_the_preceding_checked_target_write() {
    for branched in [false, true] {
        let mut f = Fixture::new(usize::from(branched), false);
        let left = f.field(f.address, 0);
        let right = f.field(f.address, 1);
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let three = f.graph.add_constant(3, f.word);
        let first = f.store(left, one);
        let old = f.store(left, two);
        suffix_with_disjoint_store(&mut f, left, right, one, three);
        let original = f.graph.clone();
        assert_eq!(f.check(old, false).stores_removed, 0);
        assert!(!f.graph.is_dead(first));
        let input = failing_input(branched);
        // Both addresses fit a 16-byte allocation (covered by check above).
        // The same raw pointer can also address only 8 valid bytes: the first
        // target access succeeds, while the unchecked disjoint range fails.
        let before = execute_with_size(&original, &input, false, 8);
        assert_access_error(&before, "memory access outside an allocation");
        assert_eq!(before.bytes, 2u64.to_le_bytes().map(Some));
        assert_eq!(execute_with_size(&f.graph, &input, false, 8), before);
    }
}

#[test]
fn disjoint_live_locals_remain_valid_across_both_control_shapes() {
    for branched in [false, true] {
        let mut f = Fixture::new(usize::from(branched), true);
        let other_root = f.local();
        let target_root = another_local(&mut f);
        let other = f.field(other_root, 0);
        let target = f.field(target_root, 0);
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let old = f.store(target, one);
        suffix_with_disjoint_store(&mut f, target, other, one, two);
        let stats = f.check(old, true);
        assert_eq!(stats.stores_removed, 1);
        assert_eq!(stats.stores_removed_across_branches, usize::from(branched));
        assert_eq!(
            execute(&f.graph, &failing_input(branched), false).result,
            Ok(SemanticOutcome::Return(Value::Scalar(2)))
        );
    }
}
