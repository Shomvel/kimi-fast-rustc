use super::*;
use crate::ValueType;
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes};

fn evaluate(predicate: P, a: u8, b: u8) -> bool {
    match predicate {
        P::Equal => a == b,
        P::NotEqual => a != b,
        P::UnsignedLess => a < b,
        P::UnsignedLessEqual => a <= b,
        P::UnsignedGreater => a > b,
        P::UnsignedGreaterEqual => a >= b,
        P::SignedLess => (a as i8) < (b as i8),
        P::SignedLessEqual => (a as i8) <= (b as i8),
        P::SignedGreater => (a as i8) > (b as i8),
        P::SignedGreaterEqual => (a as i8) >= (b as i8),
    }
}

#[test]
fn constant_implications_match_every_byte_value() {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        None,
    );
    let boolean = types.intern(TypeShape::Bool, None);
    let mut graph = FunctionGraph::new(
        "implication",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Ignore,
            c_variadic: false,
        },
        types,
    );
    // This unit exercises the mathematical helper, whose caller supplies an
    // observed proof. It does not claim Undefined is a defined source value.
    let x = graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(word),
    );
    let mut comparisons = Vec::new();
    for constant in [0u8, 1, 127, 128, 254, 255] {
        let value = graph.add_constant(u128::from(constant), word);
        for predicate in P::ALL {
            for swap in [false, true] {
                let args = if swap { [value, x] } else { [x, value] };
                let node = graph.add_node(
                    Opcode::Compare,
                    NodeId::NONE,
                    &args,
                    predicate as u32,
                    ValueType::Rust(boolean),
                );
                comparisons.push((node, predicate, constant, swap));
            }
        }
    }
    for &(known, kp, kc, ks) in &comparisons {
        for &(query, qp, qc, qs) in &comparisons {
            for truth in [false, true] {
                let result = compare(&graph, known, truth, query);
                if matches!((signed(kp), signed(qp)), (Some(a), Some(b)) if a != b) {
                    assert_eq!(result, None);
                    continue;
                }
                let mut answers = [false; 2];
                for x in 0..=u8::MAX {
                    let (a, b) = if ks { (kc, x) } else { (x, kc) };
                    if evaluate(kp, a, b) == truth {
                        let (a, b) = if qs { (qc, x) } else { (x, qc) };
                        answers[usize::from(evaluate(qp, a, b))] = true;
                    }
                }
                let expected = match answers {
                    [true, false] => Some(false),
                    [false, true] => Some(true),
                    _ => None,
                };
                // Identical RHS operands may prove an implication even when
                // the entire antecedent is impossible; that path is unreachable.
                if answers != [false, false] {
                    assert_eq!(
                        result, expected,
                        "{kp:?} {kc} swap={ks} truth={truth} => {qp:?} {qc} swap={qs}"
                    );
                }
            }
        }
    }
}

#[test]
fn integer_comparison_is_not_boolean_negation() {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        None,
    );
    let boolean = types.intern(TypeShape::Bool, None);
    let mut graph = FunctionGraph::new(
        "integer_condition",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Ignore,
            c_variadic: false,
        },
        types,
    );
    let value = graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(word),
    );
    for predicate in [P::Equal, P::NotEqual] {
        for constant in [0, 1] {
            let constant = graph.add_constant(constant, word);
            let compare = graph.add_node(
                Opcode::Compare,
                NodeId::NONE,
                &[value, constant],
                predicate as u32,
                ValueType::Rust(boolean),
            );
            assert_eq!(
                super::super::canonical_condition(&graph, compare),
                Some((compare, false))
            );
        }
    }
}
