use super::*;
use crate::analysis::refinement::CallArgumentReceipt;
use crate::verify_graph;
use kfrc_body::{Mutability, ScalarValidRange, ScalarValidity, SizeAlign};

#[test]
fn restricted_and_non_integer_types_and_inequalities_are_not_refined() {
    for kind in 0..6 {
        let mut f = Fixture::new(IntegerWidth::W64, false);
        let ty = match kind {
            0 => {
                f.graph.oracle.record_scalar_validity(
                    f.word,
                    ScalarValidity::Scalar(ScalarValidRange::new(64, 1, 17).unwrap()),
                );
                f.word
            }
            1 => f.boolean,
            2 => f
                .graph
                .oracle
                .intern(TypeShape::Char, Some(SizeAlign { size: 4, align: 4 })),
            3 => f.graph.oracle.intern(
                TypeShape::RawPointer {
                    mutability: Mutability::Not,
                    pointee: f.word,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            ),
            4 => f.graph.oracle.intern(
                TypeShape::Int {
                    width: IntegerWidth::W128,
                    signed: false,
                },
                Some(SizeAlign { size: 16, align: 8 }),
            ),
            _ => f.word,
        };
        f.word = ty;
        let mut start = match f.graph.value_type(f.graph.start) {
            ValueType::Tuple(fields) => fields.clone(),
            _ => unreachable!(),
        };
        for (slot, &arg) in f.args[..2].iter().enumerate() {
            f.graph.set_value_type(arg, ValueType::Rust(ty));
            start[slot + 2] = ValueType::Rust(ty);
        }
        f.graph
            .set_value_type(f.graph.start, ValueType::Tuple(start));
        let constant = f.graph.add_constant(1, ty);
        let condition = f.compare(
            f.args[0],
            constant,
            if kind == 5 { P::UnsignedLess } else { P::Equal },
        );
        let [yes, no] = f.branch(condition);
        f.ret(no, f.effect, constant);
        f.control = yes;
        let (_, result) = f.call("typed", &[f.args[0]]);
        f.finish(result);
        let dump = f.graph.dump();
        f.apply(0);
        assert_eq!(f.graph.dump(), dump);
    }
}

#[test]
fn separate_loads_keep_their_effects_and_are_not_equated() {
    let mut f = Fixture::new(IntegerWidth::W64, false);
    let layout = SizeAlign { size: 8, align: 8 };
    f.graph.stack_slots.push(layout);
    let pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: f.word,
        },
        Some(layout),
    );
    let address = f.graph.add_node(
        Opcode::StackSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    f.fixed(Opcode::Store, &[address, f.args[0]], 0, None);
    let (first, old) = f.fixed(Opcode::Load, &[address], 0, Some(f.word));
    let old = old.unwrap();
    let constant = f.graph.add_constant(5, f.word);
    let condition = f.compare(old, constant, P::Equal);
    let [yes, no] = f.branch(condition);
    f.ret(no, f.effect, constant);
    f.control = yes;
    f.fixed(Opcode::Store, &[address, f.args[1]], 0, None);
    let (second, new) = f.fixed(Opcode::Load, &[address], 0, Some(f.word));
    let new = new.unwrap();
    let (call, result) = f.call("snapshots", &[old, new]);
    f.finish(result);
    let before = f.apply(1);
    assert_eq!(f.graph.input(call, 1), constant);
    assert_eq!(f.graph.input(call, 2), new);
    for load in [first, second] {
        assert_eq!(f.graph.control(load), before.control(load));
        assert_eq!(f.graph.inputs(load), before.inputs(load));
    }
    for input in [0, 5] {
        assert_eq!(
            execute(&f.graph, [input, 9, 0]),
            execute(&before, [input, 9, 0])
        );
    }
    assert_eq!(
        execute(&f.graph, [5, 9, 0]).1,
        vec![("snapshots".into(), vec![5, 9])]
    );
}

#[test]
fn layout_only_and_no_candidate_paths_leave_the_graph_untouched() {
    for level in [GraphLevel::Specialized, GraphLevel::Layout] {
        let mut f = Fixture::new(IntegerWidth::W64, false);
        let (_, result) = f.call("no_guard", &[f.args[0]]);
        f.finish(result);
        f.graph.level = level;
        let dump = f.graph.dump();
        let stats = refine(&mut f.graph, MAX_FACT_QUERIES).unwrap();
        assert_eq!(stats.fact_queries, 0);
        assert_eq!(stats.arguments_refined, 0);
        assert_eq!(f.graph.dump(), dump);
    }
}

#[test]
fn indirect_calls_are_not_targets_and_variadic_abi_is_preserved() {
    for indirect in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W64, false);
        let constant = f.graph.add_constant(5, f.word);
        let condition = f.compare(f.args[0], constant, P::Equal);
        let [yes, no] = f.branch(condition);
        f.ret(no, f.effect, constant);
        f.control = yes;
        let abi = kfrc_body::FunctionAbi {
            args: vec![kfrc_body::PassMode::Direct],
            ret: kfrc_body::PassMode::Direct,
            c_variadic: true,
        };
        let (call, result) = if indirect {
            let pointer = f.graph.oracle.intern(
                TypeShape::RawPointer {
                    mutability: Mutability::Not,
                    pointee: f.word,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            let symbol = "indirect_variadic".to_owned();
            f.graph.symbols.push((symbol, 0));
            let target = f.graph.add_node(
                Opcode::SymbolAddress,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(pointer),
            );
            let payload = f.graph.call_abis.len() as u32;
            f.graph.call_abis.push(abi);
            let (call, result) = f.fixed(
                Opcode::CallIndirect,
                &[target, f.args[0]],
                payload,
                Some(f.word),
            );
            (call, result.unwrap())
        } else {
            let target = f.graph.add_callee(crate::CallTarget {
                symbol: "direct_variadic".into(),
                abi,
            });
            let (call, result) = f.fixed(Opcode::Call, &[f.args[0]], target, Some(f.word));
            (call, result.unwrap())
        };
        f.finish(result);
        let before = f.apply(usize::from(!indirect));
        assert_eq!(f.graph.callees, before.callees);
        assert_eq!(f.graph.call_abis, before.call_abis);
        assert_eq!(
            f.graph.input(call, if indirect { 2 } else { 1 }),
            if indirect { f.args[0] } else { constant }
        );
        for input in [0, 5] {
            assert_eq!(
                execute(&f.graph, [input, 0, 0]),
                execute(&before, [input, 0, 0])
            );
        }
    }
}

#[test]
fn assume_equality_begins_only_after_its_continuation() {
    // Published L erases Assume; exercise the shared fact API in its legal S stage.
    let mut f = Fixture::new(IntegerWidth::W64, false);
    f.graph.level = GraphLevel::Specialized;
    let constant = f.graph.add_constant(5, f.word);
    let condition = f.compare(f.args[0], constant, P::Equal);
    let (assume, _) = f.fixed(Opcode::Assume, &[condition], 0, None);
    let continuation = f.control;
    let (call, result) = f.call("assumed", &[f.args[0]]);
    f.finish(result);
    let cfg = ControlFlow::new(&f.graph);
    let dom = Dominators::new(&cfg);
    let mut facts = PredicateFacts::new(&f.graph, &cfg, &dom);
    assert_eq!(
        facts.integer_constant_at(f.args[0], continuation, assume),
        None
    );
    assert_eq!(
        facts.integer_constant_at(f.args[0], continuation, f.graph.control(call)),
        Some(constant)
    );
    assert_eq!(
        refine(&mut f.graph, MAX_FACT_QUERIES)
            .unwrap()
            .arguments_refined,
        0
    );
}

#[cfg(debug_assertions)]
#[test]
fn argument_receipt_rejects_stale_sibling_and_mismatched_operand_evidence() {
    let mut f = Fixture::new(IntegerWidth::W64, false);
    let constant = f.graph.add_constant(5, f.word);
    let condition = f.compare(f.args[0], constant, P::Equal);
    let [yes, no] = f.branch(condition);
    f.ret(no, f.effect, constant);
    f.control = yes;
    let (call, result) = f.call("checked", &[f.args[0]]);
    f.finish(result);
    let receipt = CallArgumentReceipt {
        call,
        slot: 1,
        original: f.args[0],
        constant,
        proof: yes,
    };
    crate::verify::call_argument_receipts(&f.graph, &[receipt]).unwrap();
    assert!(
        crate::verify::call_argument_receipts(
            &f.graph,
            &[CallArgumentReceipt {
                proof: no,
                ..receipt
            }]
        )
        .is_err()
    );
    assert!(
        crate::verify::call_argument_receipts(
            &f.graph,
            &[CallArgumentReceipt { slot: 0, ..receipt }]
        )
        .is_err()
    );
    f.graph.set_input(call, 1, f.args[1]);
    assert!(crate::verify::call_argument_receipts(&f.graph, &[receipt]).is_err());
    verify_graph(&mut f.graph).unwrap();
}
