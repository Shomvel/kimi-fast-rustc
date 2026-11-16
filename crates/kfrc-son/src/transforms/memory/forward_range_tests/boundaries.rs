use super::*;

#[test]
fn exact_and_subfield_reuse_both_require_the_complete_layout_carrier() {
    for subfield in [false, true] {
        let size = if subfield { 16 } else { 8 };
        for permission in [
            frozen(0),
            frozen(size - 1),
            frozen(size),
            frozen(size + 1),
            frozen(u64::MAX),
            ArgPermission::Frozen {
                dereferenceable: None,
            },
            ArgPermission::None,
            ArgPermission::Unique {
                dereferenceable: Some(size),
            },
        ] {
            let mut f = Fixture::new();
            let address = f.field(f.root, f.pair, 1, f.word);
            let value = f.graph.add_constant(73, f.word);
            f.store(address, value);
            let borrowed = if subfield {
                f.reference(f.root, f.pair, Mutability::Not)
            } else {
                f.reference(address, f.word, Mutability::Not)
            };
            f.call(borrowed, permission);
            let loaded = f.load(address, f.word);
            f.ret(loaded);
            f.check(usize::from(permission == frozen(size)), &[Run::default()]);
        }
    }
}

#[test]
fn a_raw_origin_is_allowed_but_only_a_shared_reference_argument_is_protected() {
    for reference in [None, Some(Mutability::Mut), Some(Mutability::Not)] {
        let mut f = Fixture::new();
        let address = f.field(f.root, f.pair, 1, f.word);
        let value = f.graph.add_constant(59, f.word);
        f.store(address, value);
        let borrowed = match reference {
            Some(mutability) => f.reference(f.root, f.pair, mutability),
            None => f.cast(f.root, f.pair),
        };
        // The verifier accepts these hand-built carriers; raw/mutable values
        // must not become shared borrows merely because a map says Frozen.
        f.call(borrowed, frozen(16));
        let loaded = f.load(address, f.word);
        f.ret(loaded);
        f.check(
            usize::from(reference == Some(Mutability::Not)),
            &[Run::default()],
        );
    }
}

#[test]
fn borrowed_pointees_need_both_source_sizedness_and_a_recorded_layout() {
    for thin_unsized in [false, true] {
        let mut f = Fixture::new();
        let pointee = f.graph.oracle.intern(
            TypeShape::Adt {
                name: "unavailable_borrow_extent".into(),
            },
            thin_unsized.then_some(SizeAlign { size: 16, align: 8 }),
        );
        if thin_unsized {
            // Source sizedness is independent of the optional layout table.
            // Even a nonzero layout cannot authenticate this forged carrier.
            f.graph.oracle.record_thin_unsized(pointee);
        }
        let address = f.field(f.root, f.pair, 1, f.word);
        let value = f.graph.add_constant(71, f.word);
        f.store(address, value);
        let borrowed = f.reference(f.root, pointee, Mutability::Not);
        f.call(borrowed, frozen(16));
        let loaded = f.load(address, f.word);
        f.ret(loaded);
        f.check(0, &[Run::default()]);
    }
}

#[test]
fn zero_sized_borrows_and_unit_loads_do_not_supply_access_validity() {
    for zero_borrow in [false, true] {
        let mut f = Fixture::new();
        let address = f.cast(f.root, f.unit);
        f.load(address, f.unit);
        let borrowed = f.reference(
            f.root,
            if zero_borrow { f.unit } else { f.pair },
            Mutability::Not,
        );
        f.call(borrowed, frozen(if zero_borrow { 0 } else { 16 }));
        f.load(address, f.unit);
        let value = f.graph.add_constant(1, f.word);
        f.ret(value);
        f.check(0, &[Run::default()]);
    }
}

#[test]
fn sibling_partial_overlap_and_outside_ranges_allow_visible_alias_mutation() {
    for shape in 0..4 {
        let mut f = Fixture::new();
        let address = f.field(f.root, f.pair, u32::from(shape != 0), f.word);
        let initial = 0x0123_4567_89ab_cdefu64;
        let replacement = 0x7766_5544_3322_1100u64;
        let value = f.graph.add_constant(u128::from(initial), f.word);
        f.store(address, value);
        let (borrowed, bytes, mutation_offset, expected) = match shape {
            0 => {
                let other = f.field(f.root, f.pair, 1, f.word);
                (
                    f.reference(other, f.word, Mutability::Not),
                    8,
                    0,
                    replacement,
                )
            }
            1 => {
                let other = f.field(f.root, f.pair, 0, f.word);
                (
                    f.reference(other, f.word, Mutability::Not),
                    8,
                    8,
                    replacement,
                )
            }
            2 => {
                let array = f.graph.oracle.intern(
                    TypeShape::Array {
                        element: f.byte,
                        length: 12,
                    },
                    Some(SizeAlign { size: 12, align: 1 }),
                );
                let expected =
                    (initial & u64::from(u32::MAX)) | ((replacement & u64::from(u32::MAX)) << 32);
                (
                    f.reference(f.root, array, Mutability::Not),
                    12,
                    12,
                    expected,
                )
            }
            _ => {
                let index = f.graph.add_constant(16, f.word);
                let other = f.index(f.root, f.byte, index);
                (
                    f.reference(other, f.word, Mutability::Not),
                    8,
                    8,
                    replacement,
                )
            }
        };
        f.call(borrowed, frozen(bytes));
        let loaded = f.load(address, f.word);
        f.ret(loaded);
        // Every write starts outside the protected borrowed interval. Partial
        // overlap (8..16 versus 0..12) changes only the unprotected suffix.
        let execution = Run {
            mutation: Some((mutation_offset, replacement)),
            ..Run::default()
        };
        f.check(0, &[execution]);
        let execution = Run {
            mutation: Some((mutation_offset, replacement)),
            ..Run::default()
        };
        assert_eq!(
            execute(&f.graph, &execution).result,
            Ok(SemanticOutcome::Return(Value::Scalar(expected)))
        );
    }
}

#[test]
fn equal_runtime_addresses_do_not_prove_equal_ssa_roots() {
    let mut f = Fixture::new();
    let address = f.field(f.other, f.pair, 1, f.word);
    let value = f.graph.add_constant(37, f.word);
    f.store(address, value);
    let borrowed = f.reference(f.root, f.pair, Mutability::Not);
    f.call(borrowed, frozen(16));
    let loaded = f.load(address, f.word);
    f.ret(loaded);
    f.check(
        0,
        &[
            Run::default(),
            Run {
                alias_roots: true,
                ..Run::default()
            },
        ],
    );
}

#[test]
fn an_indirect_call_does_not_gain_a_direct_callee_carrier() {
    let mut f = Fixture::new();
    let address = f.field(f.root, f.pair, 1, f.word);
    let value = f.graph.add_constant(43, f.word);
    f.store(address, value);
    f.graph.call_abis.push(FunctionAbi {
        args: vec![PassMode::Direct],
        ret: PassMode::Ignore,
        c_variadic: false,
    });
    f.graph
        .callee_permissions
        .insert("observe".into(), vec![frozen(16)]);
    // The raw argument imposes no shared-borrow restriction on this opaque
    // target, which can legitimately change the word before the later load.
    f.fixed(Opcode::CallIndirect, &[f.root, f.root], 0, Some(f.unit));
    let loaded = f.load(address, f.word);
    f.ret(loaded);
    let execution = Run {
        mutation: Some((8, 107)),
        ..Run::default()
    };
    f.check(0, &[execution]);
    let execution = Run {
        mutation: Some((8, 107)),
        ..Run::default()
    };
    assert_eq!(
        execute(&f.graph, &execution).result,
        Ok(SemanticOutcome::Return(Value::Scalar(107)))
    );
}

#[test]
fn loading_an_inner_pointer_does_not_extend_the_outer_borrow_to_its_pointee() {
    let mut f = Fixture::new();
    let pointer = f.pointer(f.word);
    let outer = f.cast(f.root, pointer);
    let inner = f.load(outer, pointer);
    let value = f.graph.add_constant(17, f.word);
    f.store(inner, value);
    let borrowed = f.reference(outer, pointer, Mutability::Not);
    f.call(borrowed, frozen(8));
    let loaded = f.load(inner, f.word);
    f.ret(loaded);
    let execution = Run {
        pointer_slot: true,
        mutation: Some((32, 101)),
        ..Run::default()
    };
    f.check(0, &[execution]);
    let execution = Run {
        pointer_slot: true,
        mutation: Some((32, 101)),
        ..Run::default()
    };
    assert_eq!(
        execute(&f.graph, &execution).result,
        Ok(SemanticOutcome::Return(Value::Scalar(101)))
    );
}

#[test]
fn address_ranges_keep_the_existing_canonical_index_and_overflow_limits() {
    for (width, signed, raw, dynamic, accepted) in [
        (IntegerWidth::W64, false, 0, false, true),
        (IntegerWidth::W8, false, 0, false, true),
        (IntegerWidth::W8, false, 256, false, false),
        (IntegerWidth::W16, true, 65535, false, false),
        (IntegerWidth::W64, false, u64::MAX as u128, false, false),
        (IntegerWidth::W64, false, u64::MAX as u128 / 8, false, false),
        (IntegerWidth::W64, false, 0, true, false),
    ] {
        let mut f = Fixture::new();
        let ty = f.graph.oracle.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign {
                size: u64::from(width.bits(64) / 8),
                align: u64::from(width.bits(64) / 8),
            }),
        );
        let literal = f.graph.add_constant(raw, ty);
        let index = if dynamic {
            f.fixed(Opcode::Opaque, &[literal], 0, Some(ty)).1.unwrap()
        } else {
            literal
        };
        let indexed = f.index(f.root, f.word, index);
        let address = f.cast(f.root, f.word);
        let value = f.graph.add_constant(67, f.word);
        f.store(address, value);
        let borrowed = f.reference(indexed, f.word, Mutability::Not);
        f.call(borrowed, frozen(8));
        let loaded = f.load(address, f.word);
        f.ret(loaded);
        f.check(usize::from(accepted), &[Run::default()]);
    }
}

#[test]
fn a_write_after_the_protected_call_still_clobbers_the_snapshot() {
    let mut f = Fixture::new();
    let address = f.cast(f.root, f.word);
    let first = f.load(address, f.word);
    let borrowed = f.reference(f.root, f.pair, Mutability::Not);
    f.call(borrowed, frozen(16));
    let index = f.graph.add_constant(3, f.word);
    let byte = f.index(f.root, f.byte, index);
    let replacement = f.graph.add_constant(0xee, f.byte);
    f.store(byte, replacement);
    let second = f.load(address, f.word);
    let difference = f.graph.add_node(
        Opcode::BitXor,
        NodeId::NONE,
        &[first, second],
        0,
        ValueType::Rust(f.word),
    );
    f.ret(difference);
    f.check(0, &[Run::default()]);
    assert_eq!(
        execute(&f.graph, &Run::default()).result,
        Ok(SemanticOutcome::Return(Value::Scalar(0xb4 << 24)))
    );
}
