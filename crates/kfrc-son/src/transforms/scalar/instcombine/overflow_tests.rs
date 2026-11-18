use super::*;
use crate::{
    GraphLevel, InterpretError, Interpreter, SemanticOutcome, Value, ValueType, verify_graph,
};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign};

fn graph(width: IntegerWidth, signed: bool) -> (FunctionGraph, TyId, TyId, NodeId, NodeId) {
    let mut types = RecordedTypes::new();
    let bytes = u64::from(width.bits(64) / 8);
    let word = types.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign {
            size: bytes,
            align: bytes,
        }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut graph = FunctionGraph::new(
        "constant_overflow",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Specialized;
    graph.start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[graph.start],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[graph.start],
        1,
        ValueType::Effect,
    );
    (graph, word, boolean, control, effect)
}

fn flag(graph: &mut FunctionGraph, boolean: TyId, a: NodeId, b: NodeId) -> NodeId {
    graph.add_node(
        Opcode::MulOverflow,
        NodeId::NONE,
        &[a, b],
        0,
        ValueType::Rust(boolean),
    )
}

fn outcome(graph: &FunctionGraph) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
}

#[test]
fn exhaustive_byte_and_wider_boundary_flags_match_source_width_and_signedness() {
    for signed in [false, true] {
        for width in [
            IntegerWidth::W8,
            IntegerWidth::W16,
            IntegerWidth::W32,
            IntegerWidth::W64,
            IntegerWidth::Pointer,
        ] {
            let (mut g, word, boolean, _, _) = graph(width, signed);
            let a = g.add_constant(0, word);
            let b = g.add_constant(0, word);
            let node = flag(&mut g, boolean, a, b);
            let bits = width.bits(64);
            let mask = u64::MAX >> (64 - bits);
            let sign = 1u64 << (bits - 1);
            let values: Vec<_> = if bits == 8 {
                (0..256).collect()
            } else {
                vec![
                    0,
                    1,
                    2,
                    3,
                    10,
                    sign - 2,
                    sign - 1,
                    sign,
                    sign + 1,
                    mask - 1,
                    mask,
                ]
            };
            for &left in &values {
                for &right in &values {
                    let expected = if signed {
                        let shift = 64 - bits;
                        let lhs = ((left << shift) as i64) >> shift;
                        let rhs = ((right << shift) as i64) >> shift;
                        let (_, native_overflow) = lhs.overflowing_mul(rhs);
                        let native = lhs.wrapping_mul(rhs);
                        u64::from(
                            native_overflow
                                || (bits < 64 && ((native << shift) >> shift) != native),
                        )
                    } else {
                        u64::from(left.checked_mul(right).is_none_or(|v| v > mask))
                    };
                    let actual = evaluate(&g, node, |n| {
                        if n == a {
                            Some(left)
                        } else if n == b {
                            Some(right)
                        } else {
                            None
                        }
                    });
                    assert_eq!(
                        actual,
                        fold_mul_overflow_enabled().then_some(expected),
                        "{bits} signed={signed} {left}*{right}"
                    );
                }
            }
        }
    }
}

#[test]
fn full_pipeline_prunes_constant_overflow_branches_and_keeps_the_wrapping_product() {
    for signed in [false, true] {
        for width in [
            IntegerWidth::W8,
            IntegerWidth::W16,
            IntegerWidth::W32,
            IntegerWidth::W64,
        ] {
            let bits = width.bits(64);
            let mask = u64::MAX >> (64 - bits);
            for (left, right) in [(0, 10), (1, 1), (mask, 2), (1u64 << (bits - 1), mask)] {
                let (mut g, word, boolean, control, effect) = graph(width, signed);
                let a = g.add_constant(u128::from(left), word);
                let b = g.add_constant(u128::from(right), word);
                let overflow = flag(&mut g, boolean, a, b);
                let branch = g.add_node(
                    Opcode::If,
                    control,
                    &[overflow],
                    0,
                    ValueType::Tuple(vec![ValueType::Control; 2]),
                );
                let product =
                    g.add_node(Opcode::Mul, NodeId::NONE, &[a, b], 0, ValueType::Rust(word));
                for slot in 0..2 {
                    let edge = g.add_node(
                        Opcode::Projection,
                        NodeId::NONE,
                        &[branch],
                        slot,
                        ValueType::Control,
                    );
                    let returned = if slot == 0 {
                        g.add_constant(77, word)
                    } else {
                        product
                    };
                    g.add_node(
                        Opcode::Return,
                        edge,
                        &[effect, returned],
                        0,
                        ValueType::Control,
                    );
                }
                verify_graph(&mut g).unwrap();
                let before = outcome(&g);
                crate::optimize_specialized_graph(&mut g).unwrap();
                verify_graph(&mut g).unwrap();
                assert_eq!(outcome(&g), before);
                if fold_mul_overflow_enabled() {
                    assert!(
                        !g.ids()
                            .any(|n| matches!(g.op(n), Opcode::MulOverflow | Opcode::If))
                    );
                }
            }
        }
    }
}

#[test]
fn one_constant_never_erases_a_poison_or_dynamic_operand() {
    for signed in [false, true] {
        let (mut g, word, boolean, control, effect) = graph(IntegerWidth::W64, signed);
        let zero = g.add_constant(0, word);
        let poison = g.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(word),
        );
        let node = flag(&mut g, boolean, zero, poison);
        g.add_node(
            Opcode::Return,
            control,
            &[effect, node],
            0,
            ValueType::Control,
        );
        verify_graph(&mut g).unwrap();
        assert_eq!(evaluate(&g, node, |n| constant(&g, n)), None);
        run(&mut g);
        g.compact();
        verify_graph(&mut g).unwrap();
        assert!(g.ids().any(|n| g.op(n) == Opcode::MulOverflow));
        assert!(matches!(
            outcome(&g),
            Err(InterpretError::PoisonObserved(_))
        ));
    }
}

#[test]
fn seeded_rewrite_folds_a_new_constant_overflow_flag() {
    let (mut g, word, boolean, control, effect) = graph(IntegerWidth::W64, false);
    let zero = g.add_constant(0, word);
    let ten = g.add_constant(10, word);
    let node = flag(&mut g, boolean, zero, ten);
    g.add_node(
        Opcode::Return,
        control,
        &[effect, node],
        0,
        ValueType::Control,
    );
    verify_graph(&mut g).unwrap();
    let before = outcome(&g);
    let stats = super::super::seam::run_seeded(&mut g, &[node]);
    g.resolve_edges();
    assert_eq!(outcome(&g), before);
    if fold_mul_overflow_enabled() {
        assert_eq!(stats.constants_folded, 1);
        assert_eq!(constant(&g, node), Some(0));
    } else {
        assert_eq!(stats.constants_folded, 0);
        assert_eq!(g.resolve(node), node);
    }
    g.compact();
    verify_graph(&mut g).unwrap();
}

#[test]
fn sccp_evaluates_the_flag_after_selecting_a_feasible_phi_input() {
    for poison_on_live_edge in [false, true] {
        let (mut g, word, boolean, control, effect) = graph(IntegerWidth::W64, false);
        let condition = g.add_constant(0, boolean);
        let branch = g.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let edges: Vec<_> = (0..2)
            .map(|slot| {
                let edge = g.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[branch],
                    slot,
                    ValueType::Control,
                );
                g.add_node(Opcode::Jump, edge, &[], 0, ValueType::Control)
            })
            .collect();
        let region = g.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
        let zero = g.add_constant(0, word);
        let poison = g.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(word),
        );
        let incoming = if poison_on_live_edge {
            [zero, poison]
        } else {
            [poison, zero]
        };
        let phi = g.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, incoming[0], incoming[1]],
            0,
            ValueType::Rust(word),
        );
        let ten = g.add_constant(10, word);
        let node = flag(&mut g, boolean, phi, ten);
        g.add_node(
            Opcode::Return,
            region,
            &[effect, node],
            0,
            ValueType::Control,
        );
        verify_graph(&mut g).unwrap();
        let before = outcome(&g);
        let stats = crate::transforms::control::sccp::run(&mut g);
        assert_eq!(stats.budget_bailouts, 0);
        g.resolve_edges();
        assert_eq!(outcome(&g), before);
        if fold_mul_overflow_enabled() && !poison_on_live_edge {
            assert_eq!(constant(&g, node), Some(0));
        } else {
            assert_eq!(g.resolve(node), node);
        }
        if poison_on_live_edge {
            assert!(matches!(before, Err(InterpretError::PoisonObserved(_))));
        }
        g.compact();
        verify_graph(&mut g).unwrap();
    }
}
