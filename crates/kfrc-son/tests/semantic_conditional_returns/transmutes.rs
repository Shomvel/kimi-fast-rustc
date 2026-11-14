//! The literal profitability query shares the representation certificate; the
//! original fixed conversion still checks all data and metadata components.
use super::*;
use kfrc_body::{ScalarValidRange, ScalarValidity};

fn string(f: &mut Function) -> TyId {
    let pointee = f.graph.oracle.intern(TypeShape::Str, None);
    let ty = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    f.graph.oracle.record_aggregate(
        ty,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: f.pointer,
                    offset: 0,
                },
                FieldLayout {
                    ty: f.word,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    ty
}

fn source(f: &mut Function, length: NodeId, poison_data: bool) -> (NodeId, TyId) {
    let ty = string(f);
    let data = if poison_data {
        f.node(Opcode::Undefined, &[], 0, f.pointer)
    } else {
        f.graph.add_constant(0x20000, f.pointer)
    };
    (f.node(Opcode::WidePtr, &[data, length], 0, ty), ty)
}

fn transmute(f: &mut Function, source: NodeId, ty: TyId) -> NodeId {
    let event = f.graph.add_node(
        Opcode::TransmuteValue,
        f.control,
        &[f.effect, source],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(ty),
        ]),
    );
    f.control = f.project(event, 0, ValueType::Control);
    f.effect = f.project(event, 1, ValueType::Effect);
    f.project(event, 2, ValueType::Rust(ty))
}

fn finish(f: &mut Function, callee: &FunctionGraph, lhs: NodeId, rhs: NodeId) {
    let unused = f.graph.add_constant(17, f.word);
    let result = f.call(callee, &[lhs, rhs, unused]);
    f.marker("after");
    f.ret(result);
    f.verify();
}

#[test]
fn certified_transmute_literal_splits_without_erasing_the_representation_event() {
    let callee = equality(Argument::Slice, ComparePredicate::Equal, false, true);
    let mut caller = Function::new("converted_literal", &[Argument::Slice], false);
    caller.marker("before");
    let length = caller.graph.add_constant(5, caller.word);
    let (rhs, _) = source(&mut caller, length, false);
    let target = caller.slice;
    let rhs = transmute(&mut caller, rhs, target);
    let lhs = caller.args[0];
    finish(&mut caller, &callee.graph, lhs, rhs);
    let original = caller.graph.clone();
    let (bodies, summaries) = table(callee.graph);
    assert_eq!(
        summaries.fold_calls(&mut caller.graph).unwrap().calls_split,
        1
    );
    caller.verify();
    assert_eq!(
        caller
            .graph
            .ids()
            .filter(|&n| caller.graph.op(n) == Opcode::TransmuteValue)
            .count(),
        1
    );
    for length in [0, 1, 5, 256, u64::MAX] {
        let args = [argument(Argument::Slice, length)];
        let mut before = Vec::new();
        let mut after = Vec::new();
        assert_eq!(
            execute(&original, &bodies, &args, &mut before),
            execute(&caller.graph, &bodies, &args, &mut after)
        );
        assert_eq!(before, after);
        assert_eq!(after.contains(&"checked_payload".into()), length == 5);
    }
}

#[test]
fn source_poison_still_traps_before_the_guard_when_transmuted_length_is_literal() {
    let callee = equality(Argument::Slice, ComparePredicate::Equal, false, false);
    let mut caller = Function::new("converted_poison", &[], false);
    caller.marker("before");
    let lhs = caller.literal(Argument::Slice, 3, false);
    let length = caller.graph.add_constant(5, caller.word);
    let (rhs, _) = source(&mut caller, length, true);
    let target = caller.slice;
    let rhs = transmute(&mut caller, rhs, target);
    finish(&mut caller, &callee.graph, lhs, rhs);
    let original = caller.graph.clone();
    let (bodies, summaries) = table(callee.graph);
    assert_eq!(
        summaries.fold_calls(&mut caller.graph).unwrap().calls_split,
        1
    );
    for graph in [&original, &caller.graph] {
        let mut events = Vec::new();
        assert!(matches!(
            execute(graph, &bodies, &[], &mut events),
            Err(InterpretError::PoisonObserved(_))
        ));
        assert_eq!(events, ["before"]);
    }
}

#[test]
fn consecutive_converted_literals_use_the_original_execution_context_for_all_calls() {
    let callee = equality(Argument::Slice, ComparePredicate::Equal, false, true);
    let mut caller = Function::new("several_converted_literals", &[Argument::Slice], false);
    caller.marker("before");
    let mut result = caller.graph.add_constant(0, caller.boolean);
    for length in [3, 5, 7] {
        let length = caller.graph.add_constant(length, caller.word);
        let (rhs, _) = source(&mut caller, length, false);
        let target = caller.slice;
        let rhs = transmute(&mut caller, rhs, target);
        let unused = caller.graph.add_constant(17, caller.word);
        result = caller.call(&callee.graph, &[caller.args[0], rhs, unused]);
    }
    caller.marker("after");
    caller.ret(result);
    caller.verify();
    let original = caller.graph.clone();
    let (bodies, summaries) = table(callee.graph);
    assert_eq!(
        summaries.fold_calls(&mut caller.graph).unwrap().calls_split,
        3
    );
    caller.verify();
    assert_eq!(
        caller
            .graph
            .ids()
            .filter(|&n| caller.graph.op(n) == Opcode::TransmuteValue)
            .count(),
        3
    );
    for length in [0, 3, 5, 7, 9] {
        let args = [argument(Argument::Slice, length)];
        let mut before = Vec::new();
        let mut after = Vec::new();
        assert_eq!(
            execute(&original, &bodies, &args, &mut before),
            execute(&caller.graph, &bodies, &args, &mut after)
        );
        assert_eq!(before, after);
        assert_eq!(
            after
                .iter()
                .filter(|event| event.as_str() == "checked_payload")
                .count(),
            usize::from([3, 5, 7].contains(&length))
        );
    }
}

#[test]
fn dynamic_transmuted_metadata_and_bad_certificates_do_not_supply_literal_facts() {
    for reason in [
        "dynamic",
        "missing-layout",
        "wrong-offset",
        "restricted-metadata",
        "merged-control",
    ] {
        let callee = equality(Argument::Slice, ComparePredicate::Equal, false, true);
        let mut caller = Function::new(
            "unproved_converted_literal",
            &[Argument::Slice, Argument::Word],
            false,
        );
        caller.marker("before");
        let length = if reason == "dynamic" {
            caller.args[1]
        } else {
            caller.graph.add_constant(5, caller.word)
        };
        let (rhs, source_ty) = source(&mut caller, length, false);
        if reason == "missing-layout" {
            caller.graph.oracle.aggregates.remove(&source_ty);
        } else if reason == "wrong-offset" {
            caller
                .graph
                .oracle
                .aggregates
                .get_mut(&source_ty)
                .unwrap()
                .fields[1]
                .offset = 4;
        } else if reason == "restricted-metadata" {
            caller.graph.oracle.record_scalar_validity(
                source_ty,
                ScalarValidity::Pair {
                    first: None,
                    second: Some(ScalarValidRange::new(64, 0, 255).unwrap()),
                    second_offset: 8,
                },
            );
        } else if reason == "merged-control" {
            let jump =
                caller
                    .graph
                    .add_node(Opcode::Jump, caller.control, &[], 0, ValueType::Control);
            let region =
                caller
                    .graph
                    .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
            caller.effect = caller.graph.add_node(
                Opcode::Phi,
                region,
                &[region, caller.effect],
                0,
                ValueType::Effect,
            );
            caller.control = region;
        }
        let target = caller.slice;
        let rhs = transmute(&mut caller, rhs, target);
        let lhs = caller.args[0];
        finish(&mut caller, &callee.graph, lhs, rhs);
        let (_, summaries) = table(callee.graph);
        let before = caller.graph.dump();
        assert_eq!(
            summaries.fold_calls(&mut caller.graph).unwrap().calls_split,
            0,
            "{reason}"
        );
        assert_eq!(caller.graph.dump(), before);
    }
}
