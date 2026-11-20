use super::*;

fn character() -> Fixture {
    let mut fixture = Fixture::with_shape(TypeShape::Char, 4);
    // Match the initialized scalar validity recorded by the native producer.
    fixture.graph.oracle.record_scalar_validity(
        fixture.word,
        kfrc_body::ScalarValidity::Scalar(
            kfrc_body::ScalarValidRange::new(32, 0, 0x10ffff).unwrap(),
        ),
    );
    fixture
}

fn samples(low: u64, high: u64) -> Vec<u64> {
    let mut values: Vec<_> = (0..=255)
        .chain([0xd7fe, 0xd7ff, 0xe000, 0xe001, 0x10fffe, 0x10ffff])
        .chain((0..=16).flat_map(|plane| [plane << 16, (plane << 16) + 0xffff]))
        .chain([
            low.saturating_sub(1),
            low,
            low + 1,
            high.saturating_sub(1),
            high,
            high + 1,
        ])
        .filter(|&value| char::from_u32(value as u32).is_some() && value <= 0x10ffff)
        .collect();
    values.sort_unstable();
    values.dedup();
    values
}

fn equivalent(mut graph: FunctionGraph, inputs: impl IntoIterator<Item = u64>) {
    verify_graph(&mut graph).unwrap();
    let original = graph.clone();
    assert_eq!(
        super::super::run(&mut graph),
        usize::from(super::super::characters_enabled())
    );
    crate::transforms::control::simplify::run(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    // Modular offset and masks must never acquire the restricted char type.
    for node in graph.ids() {
        if matches!(graph.op(node), Opcode::Sub | Opcode::BitAnd) {
            assert!(matches!(
                graph.oracle.shape(graph.rust_type(node).unwrap()),
                TypeShape::Int { .. }
            ));
        }
    }
    for input in inputs {
        let old =
            Interpreter::new(&original).run(&[input], &mut |_: &str, _: &[u64]| unreachable!());
        let new = Interpreter::new(&graph).run(&[input], &mut |_: &str, _: &[u64]| unreachable!());
        assert_eq!(new, old, "char={input:#x}\n{}", graph.dump());
        assert!(matches!(old, Ok(Outcome::Return(0 | 1))));
    }
}

#[test]
fn character_intervals_preserve_scalar_boundaries_and_boolean_orientations() {
    for (low, high) in [
        (0, 0),
        (9, 13),
        (48, 57),
        (65, 90),
        (97, 122),
        (0xd7ff, 0xe000),
        (0x10ffff, 0x10ffff),
        (0, 0x10ffff),
    ] {
        for swapped in [false, true] {
            for inverted in [false, true] {
                for reverse in [false, true] {
                    for strict in [false, true] {
                        if strict && (low == 0 || high == 0x10ffff) {
                            continue;
                        }
                        let mut t = character();
                        let predicates = match (swapped, strict) {
                            (false, false) => [P::UnsignedGreaterEqual, P::UnsignedLessEqual],
                            (true, false) => [P::UnsignedLessEqual, P::UnsignedGreaterEqual],
                            (false, true) => [P::UnsignedGreater, P::UnsignedLess],
                            (true, true) => [P::UnsignedLess, P::UnsignedGreater],
                        };
                        let conditions = [
                            t.compare(predicates[0], low - u64::from(strict), swapped, inverted),
                            t.compare(predicates[1], high + u64::from(strict), swapped, inverted),
                        ];
                        let first = usize::from(reverse);
                        let pass = usize::from(inverted);
                        let a = t.branch(t.control, conditions[first]);
                        let middle = t.wrapper(a[pass]);
                        let b = t.branch(middle, conditions[1 - first]);
                        let left = t.wrapper(a[1 - pass]);
                        let right = t.wrapper(b[1 - pass]);
                        t.finish(b[pass], [left, right], false);
                        equivalent(t.graph, samples(low, high));
                    }
                }
            }
        }
    }
}

#[test]
fn character_fusion_retains_observation_and_effect_barriers() {
    for rejection in 0..4 {
        let mut t = character();
        let a = t.compare(P::UnsignedGreaterEqual, 65, false, false);
        let b = t.compare(P::UnsignedLessEqual, 90, false, false);
        let first = t.branch(t.control, a);
        if rejection == 1 {
            let zero = t.graph.add_constant(0, t.word);
            t.graph.add_node(
                Opcode::DivUnsigned,
                first[0],
                &[t.input, zero],
                0,
                ValueType::Rust(t.word),
            );
        } else if rejection == 2 {
            let one = t.graph.add_constant(1, t.word);
            t.graph.add_node(
                Opcode::IntCast,
                first[0],
                &[one],
                0,
                ValueType::Rust(t.word),
            );
        } else if rejection == 3 {
            // No arithmetic rewrite of source-level character observations.
            t.graph.level = GraphLevel::Specialized;
        }
        let second = t.branch(first[0], b);
        t.finish(second[0], [first[1], second[1]], rejection == 0);
        verify_graph(&mut t.graph).unwrap();
        let before = t.graph.dump();
        assert_eq!(super::super::run(&mut t.graph), 0);
        assert_eq!(t.graph.dump(), before);
    }
}

#[test]
fn undefined_character_is_still_observed_by_the_first_test() {
    let mut t = character();
    let poison = t.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(t.word),
    );
    t.input = poison;
    let a = t.compare(P::UnsignedGreaterEqual, 48, false, false);
    let b = t.compare(P::UnsignedLessEqual, 57, false, false);
    let first = t.branch(t.control, a);
    let second = t.branch(first[0], b);
    t.finish(second[0], [first[1], second[1]], false);
    verify_graph(&mut t.graph).unwrap();
    let old = Interpreter::new(&t.graph).run(&[0], &mut |_: &str, _: &[u64]| unreachable!());
    assert!(old.is_err());
    assert_eq!(
        super::super::run(&mut t.graph),
        usize::from(super::super::characters_enabled())
    );
    crate::transforms::control::simplify::run(&mut t.graph).unwrap();
    verify_graph(&mut t.graph).unwrap();
    let new = Interpreter::new(&t.graph).run(&[0], &mut |_: &str, _: &[u64]| unreachable!());
    assert_eq!(new, old);
}

#[test]
fn ordinary_restricted_integer_lanes_keep_the_existing_rejection() {
    let mut t = Fixture::new(IntegerWidth::W32, false);
    t.graph.oracle.record_scalar_validity(
        t.word,
        kfrc_body::ScalarValidity::Scalar(
            kfrc_body::ScalarValidRange::new(32, 0, 0x10ffff).unwrap(),
        ),
    );
    let a = t.compare(P::UnsignedGreaterEqual, 48, false, false);
    let b = t.compare(P::UnsignedLessEqual, 57, false, false);
    let first = t.branch(t.control, a);
    let second = t.branch(first[0], b);
    t.finish(second[0], [first[1], second[1]], false);
    verify_graph(&mut t.graph).unwrap();
    let before = t.graph.dump();
    assert_eq!(super::super::run(&mut t.graph), 0);
    assert_eq!(t.graph.dump(), before);
}

#[test]
fn character_letter_intervals_feed_the_existing_union_rule() {
    let mut t = character();
    let a = t.compare(P::UnsignedGreaterEqual, 65, false, false);
    let b = t.compare(P::UnsignedLessEqual, 90, false, false);
    let c = t.compare(P::UnsignedGreaterEqual, 97, false, false);
    let d = t.compare(P::UnsignedLessEqual, 122, false, false);
    let first = t.branch(t.control, a);
    let second = t.branch(first[0], b);
    let rejected: Vec<_> = [first[1], second[1]]
        .into_iter()
        .map(|control| {
            t.graph
                .add_node(Opcode::Jump, control, &[], 0, ValueType::Control)
        })
        .collect();
    let retry = t.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &rejected,
        0,
        ValueType::Control,
    );
    let third = t.branch(retry, c);
    let fourth = t.branch(third[0], d);
    let accepted: Vec<_> = [second[0], fourth[0]]
        .into_iter()
        .map(|control| {
            t.graph
                .add_node(Opcode::Jump, control, &[], 0, ValueType::Control)
        })
        .collect();
    let success = t.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &accepted,
        0,
        ValueType::Control,
    );
    t.finish(success, [third[1], fourth[1]], false);
    verify_graph(&mut t.graph).unwrap();
    let original = t.graph.clone();
    let enabled = usize::from(super::super::characters_enabled());
    assert_eq!(super::super::run(&mut t.graph), 2 * enabled);
    crate::transforms::control::simplify::run(&mut t.graph).unwrap();
    assert_eq!(super::super::run(&mut t.graph), enabled);
    crate::transforms::control::simplify::run(&mut t.graph).unwrap();
    verify_graph(&mut t.graph).unwrap();
    for value in samples(65, 122) {
        let old =
            Interpreter::new(&original).run(&[value], &mut |_: &str, _: &[u64]| unreachable!());
        let new =
            Interpreter::new(&t.graph).run(&[value], &mut |_: &str, _: &[u64]| unreachable!());
        assert_eq!(new, old, "char={value:#x}");
    }
}
