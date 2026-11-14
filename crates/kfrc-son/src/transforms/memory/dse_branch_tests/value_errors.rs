use super::*;
use kfrc_body::{EnumLayout, ScalarValidRange, ScalarValidity, TagEncoding};

fn representation(f: &mut Fixture, bits: u128, ty: TyId) -> NodeId {
    let node = f.graph.add_constant(bits, ty);
    f.graph.set_opcode(node, Opcode::ConstRepr);
    node
}

fn paired_writes(
    f: &mut Fixture,
    address: NodeId,
    first: NodeId,
    last: NodeId,
    branched: bool,
) -> NodeId {
    let old = f.store(address, first);
    if branched {
        let states = f.branch(f.conditions[0]);
        for state in states {
            f.enter(state);
            f.store(address, last);
            let result = f.graph.add_constant(1, f.word);
            f.ret(result);
        }
    } else {
        f.store(address, last);
        let result = f.graph.add_constant(1, f.word);
        f.ret(result);
    }
    old
}

fn assert_undefined(execution: &Execution) {
    assert!(
        matches!(execution.result, Err(InterpretError::UndefinedBehaviour(_))),
        "{execution:?}"
    );
}

#[test]
fn invalid_bool_and_char_constrepr_errors_are_kept_in_old_and_final_writes() {
    for branched in [false, true] {
        for character in [false, true] {
            for bad_first in [false, true] {
                let mut f = Fixture::new(usize::from(branched), true);
                let ty = if character {
                    f.graph
                        .oracle
                        .intern(TypeShape::Char, Some(SizeAlign { size: 4, align: 4 }))
                } else {
                    f.boolean
                };
                let address = f.cast(f.address, ty);
                let bad = representation(&mut f, if character { 0xd800 } else { 2 }, ty);
                let good = f.graph.add_constant(if character { 65 } else { 1 }, ty);
                let (first, last) = if bad_first { (bad, good) } else { (good, bad) };
                let old = paired_writes(&mut f, address, first, last, branched);
                let original = f.graph.clone();
                assert_eq!(f.check(old, false).stores_removed, 0);
                let input = vec![Value::Scalar(0); usize::from(branched)];
                let before = execute(&original, &input, false);
                assert_undefined(&before);
                if bad_first {
                    assert_eq!(before.bytes, vec![Some(0x5a); 16]);
                } else {
                    assert_eq!(before.bytes[0], Some(if character { 65 } else { 1 }));
                }
                assert_eq!(execute(&f.graph, &input, false), before);
            }
        }
    }
}

#[test]
fn disjoint_writes_with_direct_or_nested_repr_decoding_are_observation_barriers() {
    for branched in [false, true] {
        for nested in [false, true] {
            let mut f = Fixture::new(usize::from(branched), true);
            let left = f.field(f.address, 0);
            let right = f.field(f.address, 1);
            let one = f.graph.add_constant(1, f.word);
            let two = f.graph.add_constant(2, f.word);
            let boolean = f.boolean;
            let bad = representation(&mut f, 2, boolean);
            let (other, value) = if nested {
                let value = f.graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[bad],
                    0,
                    ValueType::Rust(f.word),
                );
                (right, value)
            } else {
                (f.cast(right, boolean), bad)
            };
            let old = f.store(left, one);
            if branched {
                let states = f.branch(f.conditions[0]);
                f.enter(states[0]);
                f.store(left, two);
                f.load_return(left);
                f.enter(states[1]);
            }
            f.store(other, value);
            f.store(left, two);
            f.load_return(left);
            let original = f.graph.clone();
            assert_eq!(f.check(old, false).stores_removed, 0);
            let input = vec![Value::Scalar(0); usize::from(branched)];
            let before = execute(&original, &input, false);
            assert_undefined(&before);
            assert_eq!(before.bytes[..8], 1u64.to_le_bytes().map(Some));
            assert_eq!(execute(&f.graph, &input, false), before);
        }
    }
}

#[test]
fn repr_decoding_hidden_behind_a_field_is_not_erased() {
    for branched in [false, true] {
        let mut f = Fixture::new(usize::from(branched), true);
        let word = f.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: false,
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        let nonzero = f.graph.oracle.intern(
            TypeShape::Adt {
                name: "NonZeroForDse".into(),
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        f.graph.oracle.record_aggregate(
            nonzero,
            AggregateLayout {
                fields: vec![FieldLayout {
                    ty: word,
                    offset: 0,
                }],
                variants: vec![],
            },
        );
        f.graph.oracle.record_scalar_validity(
            nonzero,
            ScalarValidity::Scalar(ScalarValidRange::new(32, 1, u32::MAX as u128).unwrap()),
        );
        let bad = representation(&mut f, 0, nonzero);
        let field = f.graph.add_node(
            Opcode::Field,
            NodeId::NONE,
            &[bad],
            0,
            ValueType::Rust(word),
        );
        let address = f.cast(f.address, word);
        let one = f.graph.add_constant(1, word);
        let old = paired_writes(&mut f, address, field, one, branched);
        assert_eq!(f.check(old, false).stores_removed, 0);
        let input = vec![Value::Scalar(0); usize::from(branched)];
        assert_undefined(&execute(&f.graph, &input, false));
    }
}

#[test]
fn invalid_repr_address_evaluation_on_a_disjoint_path_keeps_the_earlier_memory_state() {
    for branched in [false, true] {
        let mut f = Fixture::new(usize::from(branched), true);
        let left = f.field(f.address, 0);
        let right = f.field(f.address, 1);
        let pointer = f.graph.rust_type(right).unwrap();
        let boolean = f.boolean;
        let bad = representation(&mut f, 2, boolean);
        let index = f.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[bad],
            0,
            ValueType::Rust(f.word),
        );
        let address = f.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[right, index],
            0,
            ValueType::Rust(pointer),
        );
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let old = f.store(left, one);
        if branched {
            let states = f.branch(f.conditions[0]);
            f.enter(states[0]);
            f.store(left, two);
            f.load_return(left);
            f.enter(states[1]);
        }
        f.store(address, two);
        f.store(left, two);
        f.load_return(left);
        let original = f.graph.clone();
        assert_eq!(f.check(old, false).stores_removed, 0);
        let input = vec![Value::Scalar(0); usize::from(branched)];
        let before = execute(&original, &input, false);
        assert_undefined(&before);
        assert_eq!(before.bytes[..8], 1u64.to_le_bytes().map(Some));
        assert_eq!(execute(&f.graph, &input, false), before);
    }
}

#[path = "value_errors/encoding.rs"]
mod encoding;

#[path = "value_errors/operands.rs"]
mod operands;
