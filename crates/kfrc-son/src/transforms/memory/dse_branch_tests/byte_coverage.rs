use super::*;

fn lane(f: &mut Fixture, width: IntegerWidth) -> TyId {
    let size = u64::from(width.bits(64) / 8);
    f.graph.oracle.intern(
        TypeShape::Int {
            width,
            signed: false,
        },
        Some(SizeAlign { size, align: size }),
    )
}

fn at(f: &mut Fixture, offset: u64, ty: TyId) -> NodeId {
    let byte = lane(f, IntegerWidth::W8);
    let bytes = f.cast(f.address, byte);
    let index = f.graph.add_constant(u128::from(offset), f.word);
    let pointer = f.graph.rust_type(bytes).unwrap();
    let address = f.graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[bytes, index],
        0,
        ValueType::Rust(pointer),
    );
    f.cast(address, ty)
}

fn write_bytes(f: &mut Fixture, mask: u8, reverse: bool) {
    let byte = lane(f, IntegerWidth::W8);
    for position in 0..8 {
        let position = if reverse { 7 - position } else { position };
        if mask & (1 << position) != 0 {
            let address = at(f, position, byte);
            let value = f.graph.add_constant(u128::from(0x80 + position), byte);
            f.store(address, value);
        }
    }
}

fn write_halves(f: &mut Fixture) {
    let half = lane(f, IntegerWidth::W32);
    for offset in [0, 4] {
        let address = at(f, offset, half);
        let value = f.graph.add_constant(u128::from(0x1234_5678 + offset), half);
        f.store(address, value);
    }
}

#[test]
fn all_256_byte_subsets_delete_only_a_fully_covered_old_scalar() {
    for poison in [false, true] {
        for mask in 0..=u8::MAX {
            let mut f = Fixture::new(0, true);
            let word = f.word;
            let left = f.field(f.address, 0);
            let value = if poison {
                f.poison(word)
            } else {
                f.graph.add_constant(0x1122_3344_5566_7788, word)
            };
            let old = f.store(left, value);
            write_bytes(&mut f, mask, mask & 1 != 0);
            f.load_return(left);
            let stats = f.check(old, mask == u8::MAX);
            assert_eq!(stats.stores_removed, usize::from(mask == u8::MAX));
            assert_eq!(stats.stores_removed_using_byte_ranges, stats.stores_removed);
        }
    }
}

#[test]
fn each_branch_may_cover_with_different_widths_and_byte_orders() {
    let mut f = Fixture::new(1, true);
    let left = f.field(f.address, 0);
    let one = f.graph.add_constant(1, f.word);
    let old = f.store(left, one);
    let states = f.branch(f.conditions[0]);
    f.enter(states[0]);
    write_halves(&mut f);
    f.load_return(left);
    f.enter(states[1]);
    write_bytes(&mut f, u8::MAX, true);
    f.load_return(left);
    let stats = f.check(old, true);
    assert_eq!(stats.stores_removed, 1);
    assert_eq!(stats.stores_removed_across_branches, 1);
    assert_eq!(stats.stores_removed_using_byte_ranges, 1);
}

#[test]
fn coverage_is_not_unioned_between_distinct_control_paths() {
    let mut f = Fixture::new(1, true);
    let left = f.field(f.address, 0);
    let one = f.graph.add_constant(1, f.word);
    let old = f.store(left, one);
    let states = f.branch(f.conditions[0]);
    for (state, mask) in states.into_iter().zip([0x0f, 0xf0]) {
        f.enter(state);
        write_bytes(&mut f, mask, false);
        f.load_return(left);
    }
    assert_eq!(f.check(old, false).stores_removed, 0);
}

#[test]
fn overlapping_prefix_followed_by_full_coverage_can_remove_both_dead_writes() {
    let mut f = Fixture::new(0, true);
    let left = f.field(f.address, 0);
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    let old = f.store(left, one);
    write_bytes(&mut f, 0x24, false);
    f.store(left, two);
    f.load_return(left);
    let stats = f.check(old, true);
    assert_eq!(stats.stores_removed, 3);
    assert_eq!(stats.stores_removed_using_byte_ranges, 3);
}

#[test]
fn byte_coverage_keeps_reads_calls_and_observation_failures_in_order() {
    for barrier in [Opcode::LoadValue, Opcode::Call, Opcode::RequireDefined] {
        let mut f = Fixture::new(0, true);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(0x2233, f.word);
        let old = f.store(left, one);
        write_bytes(&mut f, 0x0f, false);
        match barrier {
            Opcode::LoadValue => {
                f.fixed(barrier, &[left], Some(f.word));
            }
            Opcode::Call => {
                f.graph.add_callee(CallTarget {
                    symbol: "observe".into(),
                    abi: FunctionAbi {
                        args: vec![],
                        ret: PassMode::Direct,
                        c_variadic: false,
                    },
                });
                f.fixed(barrier, &[], Some(f.word));
            }
            Opcode::RequireDefined => {
                let poison = f.poison(f.word);
                f.fixed(barrier, &[poison], None);
            }
            _ => unreachable!(),
        }
        write_bytes(&mut f, 0xf0, false);
        f.load_return(left);
        assert_eq!(f.check(old, false).stores_removed, 0);
    }
}

#[test]
fn misaligned_overlapping_writes_keep_the_old_external_bytes_on_failure() {
    let mut f = Fixture::new(0, true);
    let left = f.field(f.address, 0);
    let half = lane(&mut f, IntegerWidth::W32);
    let one = f.graph.add_constant(0x1234_5678, f.word);
    let value = f.graph.add_constant(7, half);
    let old = f.store(left, one);
    let unaligned = at(&mut f, 1, half);
    f.store(unaligned, value);
    write_halves(&mut f);
    f.load_return(left);
    let original = f.graph.clone();
    assert_eq!(f.check(old, false).stores_removed, 0);
    let failure = execute(&original, &[], false);
    assert!(matches!(
        failure.result,
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert_eq!(failure.bytes[..8], 0x1234_5678u64.to_le_bytes().map(Some));
}

#[test]
fn raw_first_access_validation_does_not_prove_retyped_subaccess_validity() {
    let mut f = Fixture::new(0, false);
    let left = f.field(f.address, 0);
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    let first = f.store(left, one);
    let old = f.store(left, two);
    write_halves(&mut f);
    f.load_return(left);
    assert_eq!(f.check(old, false).stores_removed, 0);
    assert!(!f.graph.is_dead(first));
}
