use super::*;
use crate::{InterpretError, Interpreter, Outcome, verify_graph};

mod direct;
mod fixture;
mod flow;
mod rejections;
mod semantic;
mod transmute_bridge;

use fixture::Fixture;

fn evaluate(graph: &FunctionGraph, active: bool) -> Result<Outcome, InterpretError> {
    Interpreter::new(graph).run(&[u64::from(active)], &mut |name: &str, _: &[u64]| {
        Err(InterpretError::UnknownCallee(name.into()))
    })
}

fn same_execution(before: &FunctionGraph, after: &FunctionGraph, active: bool) {
    match (evaluate(before, active), evaluate(after, active)) {
        (Err(InterpretError::PoisonObserved(_)), Err(InterpretError::PoisonObserved(_))) => {}
        (expected, actual) => assert_eq!(expected, actual),
    }
}

#[test]
fn complete_words_keep_little_endian_bits_in_both_phi_orders_and_or_trees() {
    for bytes in [2, 4, 8] {
        for slot in [1, 2] {
            for balanced in [false, true] {
                let mut f = Fixture::new(bytes, slot, balanced, None, 1);
                verify_graph(&mut f.graph).unwrap();
                let before = f.graph.clone();
                let stats = run(&mut f.graph, 128);
                assert_eq!(stats.groups, 1, "{}", before.dump());
                assert_eq!(stats.byte_loads_removed, bytes - 1);
                assert_eq!(stats.payload_phis_removed, bytes - 1);
                verify_graph(&mut f.graph).unwrap();
                assert_eq!(
                    f.graph
                        .ids()
                        .filter(|&n| f.graph.op(n) == Opcode::Load)
                        .count(),
                    1
                );
                assert_eq!(
                    f.graph
                        .ids()
                        .filter(|&n| f.graph.op(n) == Opcode::Phi
                            && f.graph.rust_type(n) == Some(f.byte))
                        .count(),
                    0
                );
                assert_eq!(evaluate(&f.graph, true), Ok(Outcome::Return(f.expected())));
                assert_eq!(
                    evaluate(&f.graph, false),
                    Ok(Outcome::Return(fixture::SENTINEL))
                );
                same_execution(&before, &f.graph, true);
                same_execution(&before, &f.graph, false);
            }
        }
    }
}

#[test]
fn each_missing_byte_keeps_whole_word_poison_and_inactive_guard() {
    for bytes in [2, 4, 8] {
        for missing in 0..bytes {
            let mut f = Fixture::new(bytes, 2, true, Some(missing), 3);
            verify_graph(&mut f.graph).unwrap();
            let before = f.graph.clone();
            assert_eq!(run(&mut f.graph, 128).groups, 1);
            verify_graph(&mut f.graph).unwrap();
            assert!(matches!(
                evaluate(&f.graph, true),
                Err(InterpretError::PoisonObserved(_))
            ));
            assert_eq!(
                evaluate(&f.graph, false),
                Ok(Outcome::Return(fixture::SENTINEL))
            );
            same_execution(&before, &f.graph, true);
            same_execution(&before, &f.graph, false);
        }
    }
}

#[test]
fn poisoned_inactive_pointer_is_never_read() {
    let mut f = Fixture::new(8, 1, false, None, 0);
    let pointer = f.graph.rust_type(f.base).unwrap();
    let invalid = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    for &address in &f.addresses {
        f.graph.set_input(address, 0, invalid);
    }
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph, 128).groups, 1);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(
        evaluate(&f.graph, false),
        Ok(Outcome::Return(fixture::SENTINEL))
    );
    same_execution(&before, &f.graph, false);
    same_execution(&before, &f.graph, true);
}

#[test]
fn snapshot_stays_before_source_overwrite_after_the_join() {
    for slot in [1, 2] {
        let mut f = Fixture::new(8, slot, true, None, 1);
        f.overwrite_after_join();
        verify_graph(&mut f.graph).unwrap();
        let before = f.graph.clone();
        assert_eq!(run(&mut f.graph, 128).groups, 1);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(evaluate(&f.graph, true), Ok(Outcome::Return(f.expected())));
        same_execution(&before, &f.graph, true);
        same_execution(&before, &f.graph, false);
    }
}

#[test]
fn erased_zero_offset_and_word_observers_keep_the_same_result() {
    let mut f = Fixture::new(4, 1, false, None, 0);
    f.graph.set_input(f.loads[0], 1, f.base);
    // A second full-value consumer is legal; all byte observations still pass
    // through exactly the same poison-propagating complete pack.
    let zero = f.graph.add_constant(0, f.word);
    f.graph.add_node(
        Opcode::BitXor,
        NodeId::NONE,
        &[f.pack, zero],
        0,
        ValueType::Rust(f.word),
    );
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph, 128).groups, 1);
    verify_graph(&mut f.graph).unwrap();
    same_execution(&before, &f.graph, true);
}

#[test]
fn fuel_exhaustion_discards_every_tentative_plan() {
    let f = Fixture::new(8, 2, true, None, 1);
    let mut generous = f.graph.clone();
    assert_eq!(run_with_fuel(&mut generous, 128, FUEL).groups, 1);
    let mut first_success = None;
    for budget in 0..FUEL {
        let mut graph = f.graph.clone();
        let before = graph.dump();
        let stats = run_with_fuel(&mut graph, 128, budget);
        if stats.groups == 1 {
            first_success = Some(budget);
            verify_graph(&mut graph).unwrap();
            break;
        }
        assert!(stats.budget_exhausted);
        assert_eq!(stats.byte_loads_removed, 0);
        assert_eq!(graph.dump(), before, "budget {budget}");
        assert_eq!(graph.len(), f.graph.len());
    }
    assert!(first_success.is_some_and(|budget| budget < 10_000));
}

#[test]
fn code_size_budget_and_wrong_graph_level_leave_storage_untouched() {
    for bytes in [2, 4, 8] {
        for budget in 0..bytes as u64 {
            let mut f = Fixture::new(bytes, 1, false, None, 0);
            let before = f.graph.dump();
            assert_eq!(run(&mut f.graph, budget), Stats::default());
            assert_eq!(f.graph.dump(), before);
        }
        let mut f = Fixture::new(bytes, 1, false, None, 0);
        f.graph.level = GraphLevel::Specialized;
        let before = f.graph.dump();
        assert_eq!(run(&mut f.graph, 128), Stats::default());
        assert_eq!(f.graph.dump(), before);
    }
}
