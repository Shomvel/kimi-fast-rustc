use super::*;

#[test]
fn both_arms_cover_the_same_typed_place_with_shared_effects_and_disjoint_writes() {
    let mut f = Fixture::new(1, true);
    let left = f.field(f.address, 0);
    let right = f.field(f.address, 1);
    let retyped = f.cast(left, f.word);
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    let old = f.store(left, one);
    f.store(right, one);
    f.straight_jump();
    let states = f.branch(f.conditions[0]);
    for (index, state) in states.into_iter().enumerate() {
        f.enter(state);
        f.straight_jump();
        f.store(right, two);
        f.store(if index == 0 { left } else { retyped }, two);
        f.load_return(left);
    }
    let stats = f.check(old, true);
    // Both the old left write and the pre-branch right write are covered.
    assert_eq!(stats.stores_removed_across_branches, 2);
    assert_eq!(stats.branches_visited, 2);
    assert_eq!(
        execute(&f.graph, &[Value::Scalar(0)], false).result,
        Ok(SemanticOutcome::Return(Value::Scalar(2)))
    );
}

#[test]
fn one_uncovered_arm_keeps_the_old_store_atomically() {
    for load in [false, true] {
        let mut f = Fixture::new(1, true);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let old = f.store(left, one);
        let states = f.branch(f.conditions[0]);
        f.enter(states[0]);
        f.store(left, two);
        f.load_return(left);
        f.enter(states[1]);
        if load {
            f.load_return(left);
        } else {
            f.ret(one);
        }
        assert_eq!(f.check(old, false).stores_removed, 0);
    }
}

fn nested_overwrites(f: &mut Fixture, depth: usize, address: NodeId, value: NodeId) {
    if depth == 0 {
        f.store(address, value);
        f.load_return(address);
        return;
    }
    let states = f.branch(f.conditions[depth - 1]);
    f.enter(states[0]);
    f.store(address, value);
    f.load_return(address);
    f.enter(states[1]);
    nested_overwrites(f, depth - 1, address, value);
}

#[test]
fn nested_branch_coverage_has_an_atomic_branch_budget() {
    for depth in [8, 9] {
        let mut f = Fixture::new(depth, true);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let old = f.store(left, one);
        nested_overwrites(&mut f, depth, left, two);
        let stats = f.check(old, depth == 8);
        assert_eq!(stats.budget_bailouts != 0, depth == 9);
    }
}

#[test]
fn control_path_walks_stop_at_the_state_budget() {
    for jumps in [61, 62] {
        let mut f = Fixture::new(1, true);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let old = f.store(left, one);
        for _ in 0..jumps {
            f.straight_jump();
        }
        let states = f.branch(f.conditions[0]);
        f.overwrite_arms(states, left, one);
        let stats = f.check(old, jumps == 61);
        assert_eq!(stats.budget_bailouts != 0, jumps == 62);
    }
}

#[test]
fn disjoint_or_partially_overlapping_arm_writes_are_not_complete_coverage() {
    for partial in [false, true] {
        let mut f = Fixture::new(1, true);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let old = f.store(left, one);
        let states = f.branch(f.conditions[0]);
        f.enter(states[0]);
        f.store(left, two);
        f.load_return(left);
        f.enter(states[1]);
        if partial {
            let byte = f.graph.oracle.intern(
                TypeShape::Int {
                    width: IntegerWidth::W8,
                    signed: false,
                },
                Some(SizeAlign { size: 1, align: 1 }),
            );
            let bytes = f.cast(left, byte);
            let pointer = f.graph.rust_type(bytes).unwrap();
            let shifted = f.graph.add_node(
                Opcode::IndexAddr,
                NodeId::NONE,
                &[bytes, one],
                0,
                ValueType::Rust(pointer),
            );
            let value = f.graph.add_constant(3, byte);
            f.store(shifted, value);
        } else {
            let right = f.field(f.address, 1);
            f.store(right, two);
        }
        f.load_return(left);
        assert_eq!(f.check(old, false).stores_removed, 0);
    }
}

#[test]
fn same_bytes_with_a_complete_different_scalar_encoding_cover_the_old_store() {
    let mut f = Fixture::new(1, true);
    let left = f.field(f.address, 0);
    let signed = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: true,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let other_type = f.cast(left, signed);
    let one = f.graph.add_constant(1, f.word);
    let old = f.store(left, one);
    let states = f.branch(f.conditions[0]);
    f.enter(states[0]);
    f.store(left, one);
    f.load_return(left);
    f.enter(states[1]);
    let value = f.graph.add_constant(2, signed);
    f.store(other_type, value);
    f.load_return(left);
    let stats = f.check(old, true);
    assert_eq!(stats.stores_removed, 1);
    assert_eq!(stats.stores_removed_using_byte_ranges, 1);
}

#[test]
fn only_start_bool_arguments_and_canonical_constants_are_crossed() {
    for kind in 0..6 {
        let mut f = Fixture::new(1, true);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let old = f.store(left, one);
        let condition = match kind {
            0 | 1 => f.graph.add_constant(kind, f.boolean),
            2 => f.poison(f.boolean),
            3 => f.graph.add_node(
                Opcode::BitNot,
                NodeId::NONE,
                &[f.conditions[0]],
                0,
                ValueType::Rust(f.boolean),
            ),
            _ => f.graph.add_constant(kind - 2, f.boolean),
        };
        let states = f.branch(condition);
        f.overwrite_arms(states, left, one);
        let stats = f.check(old, kind < 2);
        assert_eq!(stats.stores_removed_across_branches, usize::from(kind < 2));
    }
}
