use super::*;
use ComparePredicate as P;

fn compare(f: &mut Fixture, p: P, a: NodeId, b: NodeId) -> NodeId {
    f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[a, b],
        p as u32,
        ValueType::Rust(f.boolean),
    )
}

#[test]
fn comparison_family_respects_false_edges_swap_and_negation() {
    for (known, truth, query, swapped, folds) in [
        (P::UnsignedLess, true, P::UnsignedLessEqual, false, 1),
        (P::UnsignedLess, false, P::UnsignedGreaterEqual, false, 1),
        (P::UnsignedLessEqual, true, P::UnsignedGreater, false, 1),
        (P::UnsignedLess, true, P::UnsignedGreaterEqual, true, 1),
        (P::Equal, true, P::UnsignedLessEqual, false, 1),
        (P::NotEqual, true, P::Equal, false, 1),
        (P::UnsignedLess, true, P::SignedLess, false, 0),
        (P::UnsignedLessEqual, true, P::UnsignedLess, false, 0),
    ] {
        for seam in [false, true] {
            for negate in [false, true] {
                let mut f = Fixture::new();
                let [a, b, _] = f.args;
                let condition = compare(&mut f, known, a, b);
                let condition = if negate { f.not(condition) } else { condition };
                let edges = f.branch(f.control, condition);
                let path = usize::from(!(truth ^ negate));
                f.ret(edges[1 - path], f.effect, 3);
                let (a, b) = if swapped { (b, a) } else { (a, b) };
                let query = compare(&mut f, query, a, b);
                let query = f.not(query);
                let [yes, no] = f.branch(edges[path], query);
                f.ret(yes, f.effect, 7);
                f.ret(no, f.effect, 9);
                differential(
                    &mut f,
                    &[
                        [0, 0, 0],
                        [0, 1, 0],
                        [1, 0, 0],
                        [u64::MAX, 0, 0],
                        [0, u64::MAX, 0],
                        [u64::MAX, u64::MAX, 0],
                    ],
                    folds,
                    seam,
                );
            }
        }
    }
}

#[test]
fn threshold_weakening_and_unsigned_nonzero_use_scoped_evidence() {
    for (known, bound, query, target, folds) in [
        (P::UnsignedLess, 4, P::UnsignedLess, 8, 1),
        (P::UnsignedLess, 8, P::UnsignedLess, 4, 0),
        (P::UnsignedLess, 1, P::Equal, 0, 1),
        (P::UnsignedGreater, 0, P::NotEqual, 0, 1),
        (P::SignedLess, 0, P::SignedLessEqual, u64::MAX, 1),
        (P::SignedLess, 0, P::UnsignedLess, 8, 0),
        (P::UnsignedGreater, u64::MAX - 1, P::Equal, u64::MAX, 1),
    ] {
        let mut f = Fixture::new();
        let x = f.args[0];
        let bound = f.graph.add_constant(u128::from(bound), f.word);
        let target = f.graph.add_constant(u128::from(target), f.word);
        let condition = compare(&mut f, known, x, bound);
        let [body, exit] = f.branch(f.control, condition);
        f.ret(exit, f.effect, 3);
        let query = compare(&mut f, query, x, target);
        let [yes, no] = f.branch(body, query);
        f.ret(yes, f.effect, 7);
        f.ret(no, f.effect, 9);
        differential(
            &mut f,
            &[
                [0, 0, 0],
                [1, 0, 0],
                [3, 0, 0],
                [4, 0, 0],
                [7, 0, 0],
                [8, 0, 0],
                [1 << 63, 0, 0],
                [u64::MAX - 1, 0, 0],
                [u64::MAX, 0, 0],
            ],
            folds,
            false,
        );
    }
    let mut f = Fixture::new();
    let [a, b, _] = f.args;
    let condition = f.less(a, b);
    let [body, exit] = f.branch(f.control, condition);
    f.ret(exit, f.effect, 3);
    let zero = f.graph.add_constant(0, f.word);
    let query = compare(&mut f, P::NotEqual, b, zero);
    let [yes, no] = f.branch(body, query);
    f.ret(yes, f.effect, 7);
    f.ret(no, f.effect, 9);
    differential(
        &mut f,
        &[[0, 0, 0], [0, 1, 0], [u64::MAX - 1, u64::MAX, 0]],
        1,
        false,
    );
}

#[test]
fn a_comparison_assumption_cannot_escape_to_the_next_iteration() {
    let mut f = Fixture::new();
    let entry = f.jump(f.control);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let zero = f.graph.add_constant(0, f.word);
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    let index = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, zero],
        0,
        ValueType::Rust(f.word),
    );
    let query = f.less(index, two);
    let [body, exit] = f.branch(header, query);
    f.ret(exit, f.effect, 7);
    let condition = compare(&mut f, P::UnsignedLessEqual, index, one);
    let (after, next_effect) = f.assume(body, condition);
    let next = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[index, two],
        0,
        ValueType::Rust(f.word),
    );
    let back = f.jump(after);
    f.graph.set_inputs(header, &[entry, back]);
    f.graph.set_inputs(index, &[header, zero, next]);
    let effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.effect, next_effect],
        0,
        ValueType::Effect,
    );
    let assumption = f.graph.input(after, 0);
    f.graph.set_input(assumption, 0, effect);
    let ret = f
        .graph
        .ids()
        .find(|&node| f.graph.op(node) == Opcode::Return)
        .unwrap();
    f.graph.set_input(ret, 0, effect);
    differential(&mut f, &[[0, 0, 0]], 0, false);
    assert_eq!(
        run(&f.graph, [0, 0, 0]),
        Ok(SemanticOutcome::Return(Value::Scalar(7)))
    );
}

#[test]
fn comparison_implication_keeps_the_initial_poison_observation() {
    let mut f = Fixture::new();
    let unknown = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    let condition = f.less(unknown, one);
    let [body, exit] = f.branch(f.control, condition);
    f.ret(exit, f.effect, 3);
    let query = f.less(unknown, two);
    let [yes, no] = f.branch(body, query);
    f.ret(yes, f.effect, 7);
    f.ret(no, f.effect, 9);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        run(&f.graph, [0, 0, 0]),
        Err(InterpretError::PoisonObserved(_))
    ));
    let stats = optimize_specialized_graph(&mut f.graph).unwrap();
    assert_eq!(stats.conditions_folded, 1);
    assert!(matches!(
        run(&f.graph, [0, 0, 0]),
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn a_sibling_threshold_does_not_prove_a_condition_after_the_join() {
    let mut f = Fixture::new();
    let [left, right] = f.branch(f.control, f.args[2]);
    let four = f.graph.add_constant(4, f.word);
    let eight = f.graph.add_constant(8, f.word);
    let condition = f.less(f.args[0], four);
    let [yes, no] = f.branch(left, condition);
    let jumps = [yes, no, right].map(|at| f.jump(at));
    let merge = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
    let query = f.less(f.args[0], eight);
    let [yes, no] = f.branch(merge, query);
    f.ret(yes, f.effect, 7);
    f.ret(no, f.effect, 9);
    differential(
        &mut f,
        &[
            [0, 0, 0],
            [0, 0, 1],
            [7, 0, 0],
            [7, 0, 1],
            [8, 0, 0],
            [8, 0, 1],
        ],
        0,
        false,
    );
}

#[test]
fn identical_undefined_operands_are_not_a_proof() {
    let mut f = Fixture::new();
    let unknown = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    let query = compare(&mut f, P::Equal, unknown, unknown);
    let [yes, no] = f.branch(f.control, query);
    f.ret(yes, f.effect, 7);
    f.ret(no, f.effect, 9);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        run(&f.graph, [0, 0, 0]),
        Err(InterpretError::PoisonObserved(_))
    ));
    let stats = optimize_specialized_graph(&mut f.graph).unwrap();
    assert_eq!(stats.conditions_folded, 0);
    assert!(matches!(
        run(&f.graph, [0, 0, 0]),
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn boolean_comparisons_with_constants_expose_the_underlying_guard() {
    for predicate in [P::Equal, P::NotEqual] {
        for constant in [false, true] {
            for swap in [false, true] {
                for seam in [false, true] {
                    let mut f = Fixture::new();
                    let condition = f.less(f.args[0], f.args[1]);
                    let value = f.graph.add_constant(u128::from(constant), f.boolean);
                    let (a, b) = if swap {
                        (value, condition)
                    } else {
                        (condition, value)
                    };
                    let wrapper = compare(&mut f, predicate, a, b);
                    let branches = f.branch(f.control, wrapper);
                    for at in branches {
                        let negated = f.not(condition);
                        let [yes, no] = f.branch(at, negated);
                        f.ret(yes, f.effect, 7);
                        f.ret(no, f.effect, 9);
                    }
                    differential(
                        &mut f,
                        &[[0, 0, 0], [0, 1, 0], [1, 0, 0], [u64::MAX, u64::MAX, 0]],
                        2,
                        seam,
                    );
                }
            }
        }
    }
}

#[test]
fn boolean_comparison_normalization_preserves_poison_observation() {
    let mut f = Fixture::new();
    let unknown = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.boolean),
    );
    let zero = f.graph.add_constant(0, f.boolean);
    let wrapper = compare(&mut f, P::Equal, unknown, zero);
    let [body, exit] = f.branch(f.control, wrapper);
    f.ret(exit, f.effect, 3);
    let [yes, no] = f.branch(body, unknown);
    f.ret(yes, f.effect, 7);
    f.ret(no, f.effect, 9);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        run(&f.graph, [0, 0, 0]),
        Err(InterpretError::PoisonObserved(_))
    ));
    let stats = optimize_specialized_graph(&mut f.graph).unwrap();
    assert_eq!(stats.conditions_folded, 1);
    assert!(matches!(
        run(&f.graph, [0, 0, 0]),
        Err(InterpretError::PoisonObserved(_))
    ));
}
