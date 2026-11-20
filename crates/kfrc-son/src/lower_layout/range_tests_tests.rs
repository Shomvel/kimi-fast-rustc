use super::*;
use crate::{Interpreter, Outcome, verify_graph};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    input: NodeId,
    control: NodeId,
    effect: NodeId,
}

impl Fixture {
    fn new(width: IntegerWidth, signed: bool) -> Self {
        Self::with_shape(
            TypeShape::Int { width, signed },
            u64::from(width.bits(64) / 8),
        )
    }

    fn with_shape(shape: TypeShape, bytes: u64) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            shape,
            Some(SizeAlign {
                size: bytes,
                align: bytes,
            }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let mut graph = FunctionGraph::new(
            "range",
            FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Layout;
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
            ]),
        );
        graph.start = start;
        let control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            0,
            ValueType::Control,
        );
        let effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            1,
            ValueType::Effect,
        );
        let input = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(word),
        );
        Self {
            graph,
            word,
            boolean,
            input,
            control,
            effect,
        }
    }

    fn compare(&mut self, predicate: P, constant: u64, swapped: bool, inverted: bool) -> NodeId {
        let c = self.graph.add_constant(u128::from(constant), self.word);
        let inputs = if swapped {
            [c, self.input]
        } else {
            [self.input, c]
        };
        let condition = self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &inputs,
            predicate as u32,
            ValueType::Rust(self.boolean),
        );
        if inverted {
            self.graph.add_node(
                Opcode::BitNot,
                NodeId::NONE,
                &[condition],
                0,
                ValueType::Rust(self.boolean),
            )
        } else {
            condition
        }
    }

    fn interval(&mut self, low: u64, span: u64, inverted: bool) -> NodeId {
        let low = self.graph.add_constant(u128::from(low), self.word);
        let offset = self.graph.add_node(
            Opcode::Sub,
            NodeId::NONE,
            &[self.input, low],
            0,
            ValueType::Rust(self.word),
        );
        let span = self.graph.add_constant(u128::from(span), self.word);
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[offset, span],
            if inverted {
                P::UnsignedGreater
            } else {
                P::UnsignedLessEqual
            } as u32,
            ValueType::Rust(self.boolean),
        )
    }

    fn branch(&mut self, control: NodeId, condition: NodeId) -> [NodeId; 2] {
        let branch = self.graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        [0, 1].map(|side| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                side,
                ValueType::Control,
            )
        })
    }

    fn wrapper(&mut self, control: NodeId) -> NodeId {
        let jump = self
            .graph
            .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
        self.graph
            .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control)
    }

    fn finish(&mut self, success: NodeId, failures: [NodeId; 2], different_values: bool) {
        let jumps = failures.map(|control| {
            self.graph
                .add_node(Opcode::Jump, control, &[], 0, ValueType::Control)
        });
        let region =
            self.graph
                .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
        let zero = self.graph.add_constant(0, self.word);
        let two = self.graph.add_constant(2, self.word);
        let value = self.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, zero, if different_values { two } else { zero }],
            0,
            ValueType::Rust(self.word),
        );
        let effect = self.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, self.effect, self.effect],
            0,
            ValueType::Effect,
        );
        self.graph.add_node(
            Opcode::Return,
            region,
            &[effect, value],
            0,
            ValueType::Control,
        );
        let one = self.graph.add_constant(1, self.word);
        self.graph.add_node(
            Opcode::Return,
            success,
            &[self.effect, one],
            0,
            ValueType::Control,
        );
    }
}

#[path = "char_range_tests.rs"]
mod characters;

fn differential(mut graph: FunctionGraph, inputs: impl IntoIterator<Item = u64>) {
    verify_graph(&mut graph).unwrap();
    let original = graph.clone();
    assert_eq!(super::run(&mut graph), 1, "{}", graph.dump());
    crate::transforms::control::simplify::run(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(
        graph.ids().filter(|&n| graph.op(n) == Opcode::If).count(),
        1,
        "{}",
        graph.dump()
    );
    for input in inputs {
        let before =
            Interpreter::new(&original).run(&[input], &mut |_: &str, _: &[u64]| unreachable!());
        let after =
            Interpreter::new(&graph).run(&[input], &mut |_: &str, _: &[u64]| unreachable!());
        assert_eq!(after, before, "input={input:#x}\n{}", graph.dump());
        assert!(matches!(before, Ok(Outcome::Return(0 | 1))));
    }
}

#[test]
fn all_byte_inputs_preserve_signed_unsigned_strict_swapped_and_inverted_bounds() {
    for signed in [false, true] {
        for swapped in [false, true] {
            for inverted in [false, true] {
                for upper_first in [false, true] {
                    for strict in [false, true] {
                        // In ranked order these cover both ends, a singleton,
                        // an ordinary interval, and signed-zero crossing.
                        for (low, high) in
                            [(0u64, 255u64), (1, 254), (65, 90), (127, 128), (128, 128)]
                        {
                            let mut t = Fixture::new(IntegerWidth::W8, signed);
                            let bias = if signed { 128 } else { 0 };
                            let predicates = match (signed, swapped, strict) {
                                (false, false, false) => {
                                    [P::UnsignedGreaterEqual, P::UnsignedLessEqual]
                                }
                                (false, true, false) => {
                                    [P::UnsignedLessEqual, P::UnsignedGreaterEqual]
                                }
                                (false, false, true) => [P::UnsignedGreater, P::UnsignedLess],
                                (false, true, true) => [P::UnsignedLess, P::UnsignedGreater],
                                (true, false, false) => [P::SignedGreaterEqual, P::SignedLessEqual],
                                (true, true, false) => [P::SignedLessEqual, P::SignedGreaterEqual],
                                (true, false, true) => [P::SignedGreater, P::SignedLess],
                                (true, true, true) => [P::SignedLess, P::SignedGreater],
                            };
                            if strict && (low == 0 || high == 255) {
                                continue;
                            }
                            let constants = if strict {
                                [low - 1, high + 1]
                            } else {
                                [low, high]
                            };
                            let conditions = [0, 1].map(|i| {
                                t.compare(predicates[i], constants[i] ^ bias, swapped, inverted)
                            });
                            let first = usize::from(upper_first);
                            let pass = usize::from(inverted);
                            let first_edges = t.branch(t.control, conditions[first]);
                            let middle = t.wrapper(first_edges[pass]);
                            let second_edges = t.branch(middle, conditions[1 - first]);
                            let left = t.wrapper(first_edges[1 - pass]);
                            let right = t.wrapper(second_edges[1 - pass]);
                            t.finish(second_edges[pass], [left, right], false);
                            differential(t.graph, 0..=255);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn comparison_order_controls_extension_even_when_the_lane_type_differs() {
    for signed in [false, true] {
        for (low, high) in [(0u64, 255u64), (65, 90), (127, 128), (1, 254)] {
            let mut t = Fixture::new(IntegerWidth::W8, !signed);
            let bias = if signed { 128 } else { 0 };
            let lower = t.compare(
                if signed {
                    P::SignedGreaterEqual
                } else {
                    P::UnsignedGreaterEqual
                },
                low ^ bias,
                false,
                false,
            );
            let upper = t.compare(
                if signed {
                    P::SignedLessEqual
                } else {
                    P::UnsignedLessEqual
                },
                high ^ bias,
                false,
                false,
            );
            let first = t.branch(t.control, lower);
            let second = t.branch(first[0], upper);
            t.finish(second[0], [first[1], second[1]], false);
            differential(t.graph, 0..=255);
        }
    }
}

#[test]
fn false_edges_and_negated_outside_tests_share_the_same_interval() {
    for signed in [false, true] {
        for inverted in [false, true] {
            for upper_first in [false, true] {
                let mut t = Fixture::new(IntegerWidth::W8, signed);
                let bias = if signed { 128 } else { 0 };
                let conditions = [
                    t.compare(
                        if signed {
                            P::SignedLess
                        } else {
                            P::UnsignedLess
                        },
                        65 ^ bias,
                        false,
                        inverted,
                    ),
                    t.compare(
                        if signed {
                            P::SignedGreater
                        } else {
                            P::UnsignedGreater
                        },
                        90 ^ bias,
                        false,
                        inverted,
                    ),
                ];
                let pass = usize::from(!inverted);
                let index = usize::from(upper_first);
                let first = t.branch(t.control, conditions[index]);
                let second = t.branch(first[pass], conditions[1 - index]);
                t.finish(second[pass], [first[1 - pass], second[1 - pass]], false);
                differential(t.graph, 0..=255);
            }
        }
    }
}

#[test]
fn word_intervals_keep_modular_distance_at_sign_and_width_boundaries() {
    for width in [IntegerWidth::W16, IntegerWidth::W32, IntegerWidth::W64] {
        let bits = width.bits(64);
        let max = u64::MAX >> (64 - bits);
        let sign = 1u64 << (bits - 1);
        for signed in [false, true] {
            for (low, high) in [(0, max), (1, max - 1), (sign - 1, sign), (max, max), (0, 0)] {
                let mut t = Fixture::new(width, signed);
                let bias = if signed { sign } else { 0 };
                let a = t.compare(
                    if signed {
                        P::SignedGreaterEqual
                    } else {
                        P::UnsignedGreaterEqual
                    },
                    low ^ bias,
                    false,
                    false,
                );
                let b = t.compare(
                    if signed {
                        P::SignedLessEqual
                    } else {
                        P::UnsignedLessEqual
                    },
                    high ^ bias,
                    false,
                    false,
                );
                let first = t.branch(t.control, a);
                let second = t.branch(first[0], b);
                t.finish(second[0], [first[1], second[1]], false);
                let inputs = [0, 1, max, max - 1, sign, sign - 1, low ^ bias, high ^ bias];
                differential(t.graph, inputs);
            }
        }
    }
}

#[test]
fn unequal_failure_values_and_control_anchored_uses_block_fusion() {
    for rejection in 0..4 {
        let mut t = Fixture::new(IntegerWidth::W8, false);
        let a = t.compare(P::UnsignedGreaterEqual, 65, false, false);
        let b = t.compare(P::UnsignedLessEqual, 90, false, false);
        let first = t.branch(t.control, a);
        let mut middle = first[0];
        if rejection == 1 {
            // Even unused fixed arithmetic can trap; it must not be skipped
            // for inputs that now fail the fused upper bound.
            let zero = t.graph.add_constant(0, t.word);
            t.graph.add_node(
                Opcode::DivUnsigned,
                middle,
                &[t.input, zero],
                0,
                ValueType::Rust(t.word),
            );
        } else if rejection == 2 {
            // A pinned pure value is another observable placement obligation.
            let one = t.graph.add_constant(1, t.word);
            t.graph.add_node(
                Opcode::Add,
                middle,
                &[t.input, one],
                0,
                ValueType::Rust(t.word),
            );
        } else if rejection == 3 {
            middle = t.wrapper(middle);
            let identity = t.graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &[middle, t.input],
                0,
                ValueType::Rust(t.word),
            );
            t.graph.set_input(b, 0, identity);
        }
        let second = t.branch(middle, b);
        t.finish(second[0], [first[1], second[1]], rejection == 0);
        verify_graph(&mut t.graph).unwrap();
        assert_eq!(super::run(&mut t.graph), 0);
        verify_graph(&mut t.graph).unwrap();
    }
}

#[test]
fn observations_in_specialized_graphs_remain_untouched() {
    let mut t = Fixture::new(IntegerWidth::W8, false);
    let a = t.compare(P::UnsignedGreaterEqual, 65, false, false);
    let b = t.compare(P::UnsignedLessEqual, 90, false, false);
    let first = t.branch(t.control, a);
    let second = t.branch(first[0], b);
    t.finish(second[0], [first[1], second[1]], false);
    t.graph.level = GraphLevel::Specialized;
    verify_graph(&mut t.graph).unwrap();
    assert_eq!(super::run(&mut t.graph), 0);
}

#[test]
fn paired_intervals_merge_only_when_their_sets_differ_by_one_bit() {
    for (low, high, span, expected) in [
        (65, 97, 25, 1),
        (0, 128, 127, 1),
        (122, 250, 5, 1),
        (7, 15, 0, 1),
        (65, 98, 25, 0),
        (4, 12, 4, 0),
        (0, 8, 8, 0),
        (65, 97, 32, 0),
    ] {
        for reverse in [false, true] {
            for inverted in [false, true] {
                let mut t = Fixture::new(IntegerWidth::W8, false);
                let a = t.interval(if reverse { high } else { low }, span, inverted);
                let b = t.interval(if reverse { low } else { high }, span, inverted);
                let pass = usize::from(inverted);
                let first = t.branch(t.control, a);
                let second = t.branch(first[1 - pass], b);
                t.finish(second[1 - pass], [first[pass], second[pass]], false);
                verify_graph(&mut t.graph).unwrap();
                let before = t.graph.clone();
                assert_eq!(super::run(&mut t.graph), expected);
                crate::transforms::control::simplify::run(&mut t.graph).unwrap();
                verify_graph(&mut t.graph).unwrap();
                for value in 0..=255 {
                    let old = Interpreter::new(&before)
                        .run(&[value], &mut |_: &str, _: &[u64]| unreachable!());
                    let new = Interpreter::new(&t.graph)
                        .run(&[value], &mut |_: &str, _: &[u64]| unreachable!());
                    assert_eq!(new, old, "low={low} high={high} span={span} value={value}");
                }
            }
        }
    }
}

#[test]
fn letter_bounds_share_the_widened_value_after_two_fusion_sweeps() {
    let mut t = Fixture::new(IntegerWidth::W8, false);
    let lower_a = t.compare(P::UnsignedGreaterEqual, 65, false, false);
    let upper_a = t.compare(P::UnsignedLessEqual, 90, false, false);
    let lower_b = t.compare(P::UnsignedGreaterEqual, 97, false, false);
    let upper_b = t.compare(P::UnsignedLessEqual, 122, false, false);
    let first = t.branch(t.control, lower_a);
    let second = t.branch(first[0], upper_a);
    let fail_jumps = [first[1], second[1]].map(|c| {
        t.graph
            .add_node(Opcode::Jump, c, &[], 0, ValueType::Control)
    });
    let fail_join = t.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &fail_jumps,
        0,
        ValueType::Control,
    );
    let third = t.branch(fail_join, lower_b);
    let fourth = t.branch(third[0], upper_b);
    let success_jumps = [second[0], fourth[0]].map(|c| {
        t.graph
            .add_node(Opcode::Jump, c, &[], 0, ValueType::Control)
    });
    let success_join = t.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &success_jumps,
        0,
        ValueType::Control,
    );
    t.finish(success_join, [third[1], fourth[1]], false);
    verify_graph(&mut t.graph).unwrap();
    let before = t.graph.clone();
    assert_eq!(super::run(&mut t.graph), 2);
    crate::transforms::control::simplify::run(&mut t.graph).unwrap();
    assert_eq!(super::run(&mut t.graph), 1);
    crate::transforms::control::simplify::run(&mut t.graph).unwrap();
    verify_graph(&mut t.graph).unwrap();
    assert_eq!(
        t.graph
            .ids()
            .filter(|&n| t.graph.op(n) == Opcode::If)
            .count(),
        1
    );
    for value in 0..=255 {
        let old = Interpreter::new(&before).run(&[value], &mut |_: &str, _: &[u64]| unreachable!());
        let new =
            Interpreter::new(&t.graph).run(&[value], &mut |_: &str, _: &[u64]| unreachable!());
        assert_eq!(new, old);
    }
}
