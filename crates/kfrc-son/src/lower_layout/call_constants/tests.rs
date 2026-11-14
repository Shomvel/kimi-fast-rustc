use super::*;
use crate::ComparePredicate as P;
use crate::{InterpretError, Outcome};
use kfrc_body::{IntegerWidth, TypeShape};

#[path = "fixture.rs"]
mod fixture;
use fixture::{Fixture, execute};
#[path = "boundaries.rs"]
mod boundaries;
#[path = "loops.rs"]
mod loops;

#[test]
fn scoped_equalities_preserve_results_calls_and_unrelated_observations() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        for signed in [false, true] {
            for form in 0..4 {
                let mut f = Fixture::new(width, signed);
                let (earlier, _) = f.call("before", &[f.args[0]]);
                let constant = f.graph.add_constant(0xfd, f.word);
                let mut condition = f.compare(
                    f.args[0],
                    constant,
                    if form == 0 { P::Equal } else { P::NotEqual },
                );
                if form == 2 {
                    condition = f.graph.add_node(
                        Opcode::BitNot,
                        NodeId::NONE,
                        &[condition],
                        0,
                        ValueType::Rust(f.boolean),
                    );
                } else if form == 3 {
                    // Both operand orders and Boolean == false normalization.
                    f.graph.set_inputs(condition, &[constant, f.args[0]]);
                    let zero = f.graph.add_constant(0, f.boolean);
                    condition = f.compare(condition, zero, P::Equal);
                }
                let [yes, no] = f.branch(condition);
                let (selected, bypass) = if form == 1 { (no, yes) } else { (yes, no) };
                let incoming_effect = f.effect;
                f.ret(bypass, incoming_effect, f.args[0]);
                f.control = selected;
                let (call, result) = f.call("inside", &[f.args[0], f.args[1]]);
                f.finish(result);
                let before = f.apply(1);
                assert_eq!(f.graph.input(call, 1), constant);
                assert_eq!(f.graph.input(call, 2), f.args[1]);
                assert_eq!(f.graph.input(earlier, 1), f.args[0]);
                assert_eq!(f.graph.inputs(condition), before.inputs(condition));
                assert_eq!(f.graph.callees, before.callees);
                assert_eq!(f.graph.len(), before.len());
                for input in [0, 1, 0xfc, 0xfd, 0xff] {
                    assert_eq!(
                        execute(&f.graph, [input, 7, 0]),
                        execute(&before, [input, 7, 0])
                    );
                }
            }
        }
    }
}

#[test]
fn sibling_and_merge_calls_do_not_inherit_a_branch_fact() {
    let mut f = Fixture::new(IntegerWidth::W64, false);
    let constant = f.graph.add_constant(5, f.word);
    let condition = f.compare(f.args[0], constant, P::Equal);
    let [yes, no] = f.branch(condition);
    let incoming = f.effect;
    f.control = yes;
    let (inside, _) = f.call("equal", &[f.args[0]]);
    let left_effect = f.effect;
    let left = f.jump(f.control);
    f.control = no;
    f.effect = incoming;
    let (sibling, _) = f.call("sibling", &[f.args[0]]);
    let right_effect = f.effect;
    let right = f.jump(f.control);
    let region = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[left, right],
        0,
        ValueType::Control,
    );
    f.effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, left_effect, right_effect],
        0,
        ValueType::Effect,
    );
    f.control = region;
    let (merged, result) = f.call("merged", &[f.args[0]]);
    f.finish(result);
    let before = f.apply(1);
    assert_eq!(f.graph.input(inside, 1), constant);
    for call in [sibling, merged] {
        assert_eq!(f.graph.input(call, 1), f.args[0]);
    }
    for input in [0, 5, 9] {
        assert_eq!(
            execute(&f.graph, [input, 0, 0]),
            execute(&before, [input, 0, 0])
        );
    }
}

#[test]
fn poison_remains_observed_by_the_original_branch_and_other_call_operands() {
    for poisoned in [0, 1] {
        let mut f = Fixture::new(IntegerWidth::W64, false);
        let poison = f.graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(f.word),
        );
        let value = if poisoned == 0 { poison } else { f.args[0] };
        let constant = f.graph.add_constant(5, f.word);
        let condition = f.compare(value, constant, P::Equal);
        let [yes, no] = f.branch(condition);
        let branch = f.graph.input(yes, 0);
        f.ret(no, f.effect, constant);
        f.control = yes;
        let (call, result) = f.call("observe", &[value, poison]);
        f.finish(result);
        let before = f.apply(if poisoned == 0 { 2 } else { 1 });
        let expected = Err(InterpretError::PoisonObserved(if poisoned == 0 {
            branch
        } else {
            call
        }));
        assert_eq!(execute(&before, [5, 0, 0]), (expected, vec![]));
        assert_eq!(execute(&f.graph, [5, 0, 0]), execute(&before, [5, 0, 0]));
    }
}

#[test]
fn a_query_budget_only_abandons_unproved_rewrites() {
    let mut f = Fixture::new(IntegerWidth::W64, false);
    let constant = f.graph.add_constant(3, f.word);
    let condition = f.compare(f.args[0], constant, P::Equal);
    let [yes, no] = f.branch(condition);
    f.ret(no, f.effect, constant);
    f.control = yes;
    let (_, result) = f.call("both", &[f.args[0], f.args[0]]);
    f.finish(result);
    let original = f.graph.dump();
    let empty = refine(&mut f.graph, 0).unwrap();
    assert!(empty.budget_exhausted);
    assert_eq!(empty.arguments_refined, 0);
    assert_eq!(f.graph.dump(), original);
    let before = f.graph.clone();
    let partial = refine(&mut f.graph, 1).unwrap();
    assert!(partial.budget_exhausted);
    assert_eq!(partial.arguments_refined, 1);
    for input in [0, 3, 7] {
        assert_eq!(
            execute(&f.graph, [input, 0, 0]),
            execute(&before, [input, 0, 0])
        );
    }
}

#[test]
fn cleanup_feeds_scoped_arguments_to_instcombine() {
    if !enabled() {
        return;
    }
    let mut f = Fixture::new(IntegerWidth::W64, false);
    let constant = f.graph.add_constant(5, f.word);
    let condition = f.compare(f.args[0], constant, P::Equal);
    let [yes, no] = f.branch(condition);
    f.ret(no, f.effect, constant);
    f.control = yes;
    let (_, result) = f.call("through_cleanup", &[f.args[0]]);
    f.finish(result);
    let before = f.graph.clone();
    super::super::run(&mut f.graph).unwrap();
    crate::verify_graph(&mut f.graph).unwrap();
    let call = f
        .graph
        .ids()
        .find(|&node| f.graph.op(node) == Opcode::Call)
        .unwrap();
    let argument = f.graph.input(call, 1);
    assert_eq!(f.graph.op(argument), Opcode::Const);
    assert_eq!(f.graph.constant_value(argument), 5);
    for input in [0, 5, 9] {
        assert_eq!(
            execute(&f.graph, [input, 0, 0]),
            execute(&before, [input, 0, 0])
        );
    }
}

#[test]
fn scoped_length_unlocks_only_the_existing_authenticated_comparison_rule() {
    use kfrc_body::{DataObject, Mutability, RuntimeComparison, SizeAlign};
    for trusted in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W64, false);
        let constant = f.graph.add_constant(4, f.word);
        let condition = f.compare(f.args[0], constant, P::Equal);
        let [yes, no] = f.branch(condition);
        let zero = f.graph.add_constant(0, f.word);
        f.ret(no, f.effect, zero);
        f.control = yes;
        let byte = f.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let pointer = f.graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pointers = [b"same".as_slice(), b"tame".as_slice()].map(|bytes| {
            let index = f.graph.symbols.len() as u32;
            let symbol = format!("bytes_{index}");
            f.graph.symbols.push((symbol.clone(), 0));
            f.graph.data_objects.push(DataObject {
                symbol,
                bytes: bytes.to_vec(),
                align: 1,
                ..Default::default()
            });
            f.graph.add_node(
                Opcode::SymbolAddress,
                NodeId::NONE,
                &[],
                index,
                ValueType::Rust(pointer),
            )
        });
        let integer = f.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: true,
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        let target = if trusted {
            f.graph.add_runtime_comparison(RuntimeComparison::Memcmp)
        } else {
            f.graph.add_callee(crate::CallTarget {
                symbol: "memcmp".into(),
                abi: RuntimeComparison::Memcmp.abi(),
            })
        };
        let (call, result) = f.fixed(
            Opcode::Call,
            &[pointers[0], pointers[1], f.args[0]],
            target,
            Some(integer),
        );
        let zero = f.graph.add_constant(0, integer);
        let equal = f.compare(result.unwrap(), zero, P::Equal);
        f.finish(equal);
        let before = f.apply(1);
        crate::transforms::scalar::instcombine::run(&mut f.graph);
        assert_eq!(
            f.graph.runtime_comparison(f.graph.payload(call)),
            trusted.then_some(RuntimeComparison::Bcmp)
        );
        assert_eq!(
            f.graph.callees[target as usize],
            before.callees[target as usize]
        );
        for input in [0, 4, 5] {
            assert_eq!(
                execute(&f.graph, [input, 0, 0]),
                execute(&before, [input, 0, 0])
            );
        }
        assert_eq!(execute(&f.graph, [4, 0, 0]).0, Ok(Outcome::Return(0)));
    }
}
