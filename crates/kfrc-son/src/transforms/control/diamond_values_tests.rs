use super::*;
use crate::{ArithmeticFlags, Interpreter, verify_graph};
use kfrc_body::{IntegerWidth, SizeAlign};

#[derive(Clone, Copy)]
enum Guard {
    Overflow,
    Equal(u64),
}

struct Fixture {
    graph: FunctionGraph,
    expression: NodeId,
    phi: NodeId,
    mask: u64,
}

fn fixture(
    width: IntegerWidth,
    operation: Opcode,
    amount: u64,
    guard: Guard,
    inverted: bool,
    swapped: bool,
) -> Fixture {
    let mut graph = super::super::tests::empty_diamond(2, false);
    let argument = graph
        .ids()
        .find(|&node| {
            graph.op(node) == Opcode::Projection
                && graph.input(node, 0) == graph.start
                && graph.payload(node) == 2
        })
        .unwrap();
    let branch = graph.ids().find(|&n| graph.op(n) == Opcode::If).unwrap();
    let boolean = graph.rust_type(graph.input(branch, 0)).unwrap();
    let ret = graph
        .ids()
        .find(|&n| graph.op(n) == Opcode::Return)
        .unwrap();
    let region = graph.control(ret);
    let bits = width.bits(64);
    let ty = graph.oracle.intern(
        TypeShape::Int {
            width,
            signed: false,
        },
        Some(SizeAlign {
            size: u64::from(bits / 8),
            align: u64::from(bits / 8),
        }),
    );
    let source = graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[argument],
        0,
        ValueType::Rust(ty),
    );
    let constant = graph.add_constant(u128::from(amount), ty);
    let expression = graph.add_node(
        operation,
        NodeId::NONE,
        &[source, constant],
        0,
        ValueType::Rust(ty),
    );
    let mask = u64::MAX >> (64 - bits);
    let (mut a, mut b, mut predicate, fallback) = match guard {
        Guard::Equal(value) => {
            let limit = graph.add_constant(u128::from(value), ty);
            let fallback = instcombine::evaluate(&graph, expression, |node| {
                if node == source {
                    Some(value)
                } else {
                    instcombine::constant(&graph, node)
                }
            })
            .unwrap();
            (source, limit, P::Equal, fallback)
        }
        Guard::Overflow => {
            // The checked sum and selected sum need not share an SSA node.
            let sum = graph.add_node(
                operation,
                NodeId::NONE,
                &[source, constant],
                0,
                ValueType::Rust(ty),
            );
            if operation == Opcode::Add {
                (sum, source, P::UnsignedLess, 0)
            } else {
                (sum, source, P::UnsignedGreater, mask)
            }
        }
    };
    if swapped {
        std::mem::swap(&mut a, &mut b);
        predicate = match predicate {
            P::UnsignedLess => P::UnsignedGreater,
            P::UnsignedGreater => P::UnsignedLess,
            other => other,
        };
        let mut edges = graph.inputs(region).to_vec();
        edges.reverse();
        graph.set_inputs(region, &edges);
    }
    let mut condition = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[a, b],
        predicate as u32,
        ValueType::Rust(boolean),
    );
    if inverted {
        condition = graph.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[condition],
            0,
            ValueType::Rust(boolean),
        );
    }
    graph.set_input(branch, 0, condition);
    let fallback = graph.add_constant(u128::from(fallback), ty);
    let values = if inverted ^ swapped {
        [expression, fallback]
    } else {
        [fallback, expression]
    };
    let phi = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, values[0], values[1]],
        0,
        ValueType::Rust(ty),
    );
    graph.set_input(ret, 1, phi);
    Fixture {
        graph,
        expression,
        phi,
        mask,
    }
}

fn compare(mut f: Fixture, expected_changes: usize) -> FunctionGraph {
    verify_graph(&mut f.graph).unwrap();
    let original = f.graph.clone();
    assert_eq!(
        super::super::run(&mut f.graph),
        expected_changes,
        "{}",
        f.graph.dump()
    );
    verify_graph(&mut f.graph).unwrap();
    let inputs = (0..=255).chain([f.mask / 2, f.mask / 2 + 1, f.mask - 1, f.mask]);
    for input in inputs {
        let run = |graph: &FunctionGraph| {
            Interpreter::new(graph).run(&[input], &mut |_: &str, _: &[u64]| unreachable!())
        };
        assert_eq!(
            run(&f.graph),
            run(&original),
            "input={input}, {}",
            f.graph.dump()
        );
    }
    f.graph
}

#[test]
fn unit_overflow_diamonds_preserve_all_byte_inputs_and_word_boundaries() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        for operation in [Opcode::Add, Opcode::Sub] {
            for inverted in [false, true] {
                for swapped in [false, true] {
                    compare(
                        fixture(width, operation, 1, Guard::Overflow, inverted, swapped),
                        1,
                    );
                }
            }
        }
    }
}

#[test]
fn equality_diamonds_substitute_nested_wrapping_integer_cones() {
    for operation in [Opcode::Add, Opcode::Sub, Opcode::Mul, Opcode::BitXor] {
        for inverted in [false, true] {
            for swapped in [false, true] {
                let mut f = fixture(
                    IntegerWidth::W8,
                    operation,
                    3,
                    Guard::Equal(37),
                    inverted,
                    swapped,
                );
                let ty = f.graph.rust_type(f.expression).unwrap();
                let one = f.graph.add_constant(1, ty);
                let nested = f.graph.add_node(
                    Opcode::Add,
                    NodeId::NONE,
                    &[f.expression, one],
                    0,
                    ValueType::Rust(ty),
                );
                for slot in 1..=2 {
                    let old = f.graph.input(f.phi, slot);
                    let new = if old == f.expression {
                        nested
                    } else {
                        f.graph
                            .add_constant((f.graph.constant_value(old) + 1) & 255, ty)
                    };
                    f.graph.set_input(f.phi, slot, new);
                }
                compare(f, 1);
            }
        }
    }
}

#[test]
fn differing_fallbacks_and_nonunit_overflow_are_not_folded() {
    let mut wrong = fixture(
        IntegerWidth::W8,
        Opcode::Add,
        1,
        Guard::Overflow,
        false,
        false,
    );
    let ty = wrong.graph.rust_type(wrong.expression).unwrap();
    let one = wrong.graph.add_constant(1, ty);
    wrong.graph.set_input(wrong.phi, 1, one);
    compare(wrong, 0);
    compare(
        fixture(
            IntegerWidth::W8,
            Opcode::Add,
            2,
            Guard::Overflow,
            false,
            false,
        ),
        0,
    );
}

#[test]
fn overflow_contracts_are_relaxed_only_for_the_new_layout_use() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        for operation in [Opcode::Add, Opcode::Sub] {
            for flags in [ArithmeticFlags::NUW, ArithmeticFlags::NSW] {
                let sign = 1u64 << (width.bits(64) - 1);
                let guard = if flags == ArithmeticFlags::NUW {
                    Guard::Overflow
                } else {
                    Guard::Equal(if operation == Opcode::Add {
                        sign - 1
                    } else {
                        sign
                    })
                };
                let mut f = fixture(width, operation, 1, guard, true, true);
                f.graph.set_payload(f.expression, flags);
                let original = f.expression;
                let phi = f.phi;
                let graph = compare(f, 1);
                let replacement = graph.resolve(phi);
                assert_ne!(replacement, original);
                assert_eq!(graph.op(replacement), operation);
                assert_eq!(graph.payload(replacement), 0);
                assert_eq!(graph.payload(original), flags);
            }
        }
    }
}

#[test]
fn shared_original_contracts_and_specialized_poison_observations_are_preserved() {
    let mut f = fixture(
        IntegerWidth::W8,
        Opcode::Add,
        1,
        Guard::Overflow,
        false,
        false,
    );
    f.graph.set_payload(f.expression, ArithmeticFlags::NUW);
    let ret = f
        .graph
        .ids()
        .find(|&n| f.graph.op(n) == Opcode::Return)
        .unwrap();
    let ty = f.graph.rust_type(f.expression).unwrap();
    let sum = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[f.phi, f.expression],
        0,
        ValueType::Rust(ty),
    );
    f.graph.set_input(ret, 1, sum);
    // MAX still observes poison through the original expression, even though
    // the selected Phi now has a total wrapping replacement.
    compare(f, 1);

    let mut f = fixture(
        IntegerWidth::W8,
        Opcode::Add,
        1,
        Guard::Overflow,
        false,
        false,
    );
    f.graph.level = GraphLevel::Specialized;
    f.graph.set_payload(f.expression, ArithmeticFlags::NUW);
    compare(f, 0);
}

#[test]
fn unrelated_poison_and_anchored_expressions_stop_substitution() {
    for anchored in [false, true] {
        let mut f = fixture(
            IntegerWidth::W8,
            Opcode::Add,
            1,
            Guard::Overflow,
            false,
            false,
        );
        if anchored {
            let branch = f
                .graph
                .ids()
                .find(|&n| f.graph.op(n) == Opcode::If)
                .unwrap();
            f.graph.set_control(f.expression, f.graph.control(branch));
        } else {
            let ty = f.graph.rust_type(f.expression).unwrap();
            let poison =
                f.graph
                    .add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Rust(ty));
            f.graph.set_input(f.expression, 1, poison);
        }
        compare(f, 0);
    }
}
