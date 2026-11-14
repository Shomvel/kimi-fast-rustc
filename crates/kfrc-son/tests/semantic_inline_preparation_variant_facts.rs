//! Successful enum payloads supply conditional facts without a caller cleanup.
#[path = "semantic_inline_preparation/fixture.rs"]
mod fixture;
use fixture::*;
use kfrc_body::fx_hash::HashMap;
use kfrc_body::{SemanticAggregate, SizeAlign, TyId, TypeShape};
use kfrc_son::{
    ArithmeticFlags, ComparePredicate, FunctionGraph, InlineBudget, InterpretError, NodeId, Opcode,
    SemanticOutcome, Value, ValueType, inline_calls, verify_graph,
};

fn optional(f: &mut Function) -> TyId {
    let ty = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "PreparationOptional".into(),
        },
        None,
    );
    f.graph.oracle.record_semantic_aggregate(
        ty,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![], vec![f.word]],
        },
    );
    f.graph.oracle.record_semantic_enum(ty, vec![0, 1]);
    ty
}

fn pair(f: &mut Function) -> TyId {
    let ty = f.graph.oracle.intern(
        TypeShape::Tuple(vec![f.word; 2]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    f.graph.oracle.record_semantic_aggregate(
        ty,
        SemanticAggregate {
            fields: vec![f.word; 2],
            variants: vec![],
        },
    );
    ty
}

fn source(aggregate: bool) -> (HashMap<String, FunctionGraph>, FunctionGraph) {
    let leaf = leaf();
    let mut source = guarded("variant_fact_guard", &leaf.graph);
    if aggregate {
        let pair = pair(&mut source);
        let ValueType::Tuple(mut outputs) = source.graph.value_type(source.graph.start).clone()
        else {
            unreachable!()
        };
        outputs[3] = ValueType::Rust(pair);
        source
            .graph
            .set_value_type(source.graph.start, ValueType::Tuple(outputs));
        source
            .graph
            .set_value_type(source.args[1], ValueType::Rust(pair));
        let before = source.graph.len();
        let selected = source.node(Opcode::Field, &[source.args[1]], 0, source.word);
        for index in 0..before {
            let node = NodeId(index as u32);
            for slot in 0..source.graph.inputs(node).len() {
                if source.graph.input(node, slot) == source.args[1] {
                    source.graph.set_input(node, slot, selected);
                }
            }
        }
        source.verify();
    }
    let graphs = HashMap::from_iter([
        (leaf.graph.symbol.clone(), leaf.graph),
        (source.graph.symbol.clone(), source.graph.clone()),
    ]);
    (graphs, source.graph)
}

#[derive(Clone, Copy)]
enum Arm {
    Inactive,
    Literal(u64),
    Difference(u64, u64),
    Dynamic,
    Unknown,
    Deep,
    Loop,
    Poison,
}

fn program(
    arms: &[Arm],
    poison_sibling: bool,
) -> (Function, HashMap<String, FunctionGraph>, NodeId) {
    let (graphs, source) = source(poison_sibling);
    let mut f = Function::new("variant_fact_caller", 2);
    let enumeration = optional(&mut f);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut values = Vec::new();
    for arm in arms {
        let payload = match *arm {
            Arm::Inactive => {
                values.push(f.node(Opcode::MakeVariant, &[], 0, enumeration));
                continue;
            }
            Arm::Unknown => {
                values.push(f.node(Opcode::Undefined, &[], 0, enumeration));
                continue;
            }
            Arm::Loop => {
                let initial = f.constant(32);
                let initial = f.node(Opcode::MakeVariant, &[initial], 1, enumeration);
                let entry = f
                    .graph
                    .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
                let header = f.graph.add_node(
                    Opcode::Region,
                    NodeId::NONE,
                    &[entry, entry],
                    0,
                    ValueType::Control,
                );
                let phi = f.node(Opcode::Phi, &[header, initial, initial], 0, enumeration);
                f.graph.set_input(phi, 2, phi);
                let zero = f.constant(0);
                let condition = f.node(
                    Opcode::Compare,
                    &[f.args[1], zero],
                    ComparePredicate::Equal as u32,
                    boolean,
                );
                let branch = f.graph.add_node(
                    Opcode::If,
                    header,
                    &[condition],
                    0,
                    ValueType::Tuple(vec![ValueType::Control; 2]),
                );
                let back = projection(&mut f.graph, branch, 1, ValueType::Control);
                let back = f
                    .graph
                    .add_node(Opcode::Jump, back, &[], 0, ValueType::Control);
                f.graph.set_input(header, 1, back);
                f.control = projection(&mut f.graph, branch, 0, ValueType::Control);
                values.push(phi);
                continue;
            }
            Arm::Literal(value) => f.constant(value),
            Arm::Difference(a, b) => {
                let a = f.constant(a);
                let b = f.constant(b);
                f.node(Opcode::Sub, &[a, b], 0, f.word)
            }
            Arm::Dynamic => f.args[1],
            Arm::Poison => {
                let a = f.constant(0);
                let b = f.constant(u64::MAX - 31);
                f.node(Opcode::Sub, &[a, b], ArithmeticFlags::NUW, f.word)
            }
            Arm::Deep => {
                let mut value = f.constant(32);
                let ty = pair(&mut f);
                let zero = f.constant(0);
                for _ in 0..64 {
                    let aggregate = f.node(Opcode::Aggregate, &[value, zero], 0, ty);
                    value = f.node(Opcode::Field, &[aggregate], 0, f.word);
                }
                value
            }
        };
        values.push(f.node(Opcode::MakeVariant, &[payload], 1, enumeration));
    }
    let mut jumps = Vec::new();
    for index in 0..arms.len() {
        let control = if index + 1 == arms.len() {
            f.control
        } else {
            let index = f.constant(index as u64);
            let condition = f.node(
                Opcode::Compare,
                &[f.args[0], index],
                ComparePredicate::Equal as u32,
                boolean,
            );
            let branch = f.graph.add_node(
                Opcode::If,
                f.control,
                &[condition],
                0,
                ValueType::Tuple(vec![ValueType::Control; 2]),
            );
            f.control = projection(&mut f.graph, branch, 1, ValueType::Control);
            projection(&mut f.graph, branch, 0, ValueType::Control)
        };
        jumps.push(
            f.graph
                .add_node(Opcode::Jump, control, &[], 0, ValueType::Control),
        );
    }
    let region = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
    let mut incoming = vec![region];
    incoming.extend(values);
    let value = f.node(Opcode::Phi, &incoming, 0, enumeration);
    f.graph.variant_switches.push(vec![1]);
    let branch = f.graph.add_node(
        Opcode::VariantSwitch,
        region,
        &[value],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    f.control = projection(&mut f.graph, branch, 1, ValueType::Control);
    let inactive = f.constant(777);
    f.ret(inactive);
    f.control = projection(&mut f.graph, branch, 0, ValueType::Control);
    let witness = f.graph.add_node(
        Opcode::VariantWitness,
        f.control,
        &[value, f.control],
        1,
        ValueType::Witness,
    );
    let field = f.graph.intern_variant_field(1, 0);
    let selected = f.node(Opcode::VariantField, &[value, witness], field, f.word);
    let argument = if poison_sibling {
        let pair = pair(&mut f);
        let poison = f.node(Opcode::Undefined, &[], 0, f.word);
        f.node(Opcode::Aggregate, &[selected, poison], 0, pair)
    } else {
        selected
    };
    let result = f.call(&source, &[f.args[1], argument]);
    f.ret(result);
    f.verify();
    (f, graphs, selected)
}

#[test]
fn successful_payloads_refine_single_and_equal_matching_arms() {
    for arms in [
        vec![Arm::Inactive, Arm::Difference(40, 8)],
        vec![Arm::Literal(32), Arm::Difference(96, 64), Arm::Inactive],
        vec![Arm::Difference(0, u64::MAX - 31), Arm::Literal(32)],
    ] {
        let (mut f, graphs, _) = program(&arms, false);
        let before = f.graph.clone();
        let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
        assert!(stats.specializations_prepared > 0, "{stats:?}");
        assert!(!calls(&f.graph).contains(&"variant_fact_guard".to_owned()));
        f.verify();
        for selector in 0..arms.len() as u64 {
            equivalent(
                &before,
                &f.graph,
                &graphs,
                &[Value::Scalar(selector), Value::Scalar(9)],
            );
        }
    }
}

#[test]
fn differing_unknown_and_exhausted_queries_do_not_refine() {
    for arms in [
        vec![Arm::Literal(32), Arm::Literal(33)],
        vec![Arm::Literal(32), Arm::Dynamic],
        vec![Arm::Literal(32), Arm::Unknown],
        vec![Arm::Inactive, Arm::Deep],
        vec![Arm::Inactive; 64],
    ] {
        let (mut f, graphs, _) = program(&arms, false);
        let before = f.graph.clone();
        let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
        assert_eq!(stats.specializations_prepared, 0, "{stats:?}");
        assert!(calls(&f.graph).contains(&"variant_fact_guard".to_owned()));
        f.verify();
        for selector in 0..arms.len() as u64 {
            equivalent(
                &before,
                &f.graph,
                &graphs,
                &[Value::Scalar(selector), Value::Scalar(9)],
            );
        }
    }
}

#[test]
fn inactive_paths_stay_inactive_and_original_arguments_stay_observed() {
    let (mut f, graphs, _) = program(&[Arm::Inactive, Arm::Difference(40, 8)], true);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    assert!(stats.specializations_prepared > 0);
    f.verify();
    for selector in [0, 1] {
        equivalent(
            &before,
            &f.graph,
            &graphs,
            &[Value::Scalar(selector), Value::Scalar(9)],
        );
    }
    assert_eq!(
        execute(&f.graph, &graphs, &[Value::Scalar(0), Value::Scalar(9)]).0,
        Ok(SemanticOutcome::Return(Value::Scalar(777)))
    );
    assert!(matches!(
        execute(&f.graph, &graphs, &[Value::Scalar(1), Value::Scalar(9)]).0,
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn recursive_phi_arms_cannot_bootstrap_a_scalar_fact() {
    let (mut f, graphs, _) = program(&[Arm::Loop], false);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    assert_eq!(stats.specializations_prepared, 0);
    assert!(calls(&f.graph).contains(&"variant_fact_guard".to_owned()));
    f.verify();
    equivalent(
        &before,
        &f.graph,
        &graphs,
        &[Value::Scalar(0), Value::Scalar(0)],
    );
}

#[test]
fn a_conditional_payload_fact_retains_its_arithmetic_poison() {
    let (mut f, graphs, _) = program(&[Arm::Inactive, Arm::Poison], false);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    assert!(stats.specializations_prepared > 0);
    f.verify();
    for selector in [0, 1] {
        equivalent(
            &before,
            &f.graph,
            &graphs,
            &[Value::Scalar(selector), Value::Scalar(9)],
        );
    }
    assert!(matches!(
        execute(&f.graph, &graphs, &[Value::Scalar(1), Value::Scalar(9)]).0,
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn mismatched_enum_or_variant_witnesses_are_rejected_before_preparation() {
    for wrong_enum in [false, true] {
        let (mut f, graphs, selected) = program(&[Arm::Inactive, Arm::Literal(32)], false);
        let witness = f.graph.input(selected, 1);
        if wrong_enum {
            let ty = f.graph.rust_type(f.graph.input(selected, 0)).unwrap();
            let payload = f.constant(32);
            let other = f.node(Opcode::MakeVariant, &[payload], 1, ty);
            f.graph.set_input(witness, 0, other);
        } else {
            f.graph.set_payload(witness, 0);
        }
        assert!(verify_graph(&mut f.graph).is_err());
        assert!(inline_calls(&mut f.graph, &graphs, InlineBudget::default()).is_err());
    }
}
