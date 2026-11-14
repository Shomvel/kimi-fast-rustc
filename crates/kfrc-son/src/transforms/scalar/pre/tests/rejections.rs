use super::*;

#[test]
fn dead_expression_existence_and_post_merge_uses_are_not_reuse_evidence() {
    no_change(
        Fixture::new(Opcode::Add, [false, false], false),
        Limits::default(),
    );
    let mut f = Fixture::new(Opcode::Add, [false, false], false);
    let tuple = f
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![f.word; 2]), None);
    let output = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[f.result, f.existing[0]],
        0,
        ValueType::Rust(tuple),
    );
    f.graph.set_input(f.ret, 1, output);
    no_change(f, Limits::default());
}

#[test]
fn opcode_payload_type_and_operand_order_are_exact() {
    let mut f = Fixture::new(Opcode::Add, [true, false], false);
    f.graph.set_opcode(f.existing[0], Opcode::Sub);
    no_change(f, Limits::default());
    no_change(
        Fixture::new(Opcode::Sub, [true, false], true),
        Limits::default(),
    );
    for flag in [ArithmeticFlags::NUW, ArithmeticFlags::NSW] {
        let mut f = Fixture::new(Opcode::Add, [true, false], false);
        f.graph.set_payload(f.result, flag);
        no_change(f, Limits::default());
    }
    for op in [Opcode::DivUnsigned, Opcode::RemUnsigned] {
        let mut f = Fixture::new(Opcode::Add, [true, false], false);
        f.graph.set_opcode(f.result, op);
        f.graph.set_control(f.result, f.region);
        no_change(f, Limits::default());
    }
    let mut f = Fixture::new(Opcode::Compare, [false, false], false);
    // Build a valid compare fixture from the integer operation skeleton.
    for node in [f.existing[0], f.existing[1], f.result] {
        f.graph.set_value_type(node, ValueType::Rust(f.boolean));
        f.graph.set_payload(node, ComparePredicate::Equal as u32);
    }
    f.observe_at_entry(f.existing[0]);
    f.graph
        .set_payload(f.result, ComparePredicate::NotEqual as u32);
    no_change(f, Limits::default());

    let mut f = Fixture::new(Opcode::Add, [true, false], false);
    let signed = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: true,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let unsigned = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    for (node, input) in [
        (f.existing[0], f.args[1]),
        (f.existing[1], f.args[2]),
        (f.result, f.phi),
    ] {
        f.graph.set_opcode(node, Opcode::IntCast);
        f.graph.set_inputs(node, &[input]);
        f.graph.set_value_type(
            node,
            ValueType::Rust(if node == f.result { unsigned } else { signed }),
        );
    }
    no_change(f, Limits::default());
}

#[test]
fn two_direct_phis_from_different_regions_are_not_translated_together() {
    let mut f = Fixture::new(Opcode::Add, [true, false], false);
    let branch = f.graph.add_node(
        Opcode::If,
        f.region,
        &[f.args[0]],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let mut preds = Vec::new();
    for side in 0..2 {
        let control = projection(&mut f.graph, branch, side, ValueType::Control);
        preds.push(
            f.graph
                .add_node(Opcode::Jump, control, &[], 0, ValueType::Control),
        );
    }
    let second = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &preds, 0, ValueType::Control);
    let phi = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[second, f.delta, f.args[1]],
        0,
        ValueType::Rust(f.word),
    );
    f.graph.set_input(f.result, 1, phi);
    f.graph.set_control(f.ret, second);
    no_change(f, Limits::default());
}

#[test]
fn loop_carried_phis_are_not_treated_as_acyclic_merge_values() {
    let mut f = Fixture::new(Opcode::Add, [false, false], false);
    let kept = [
        f.graph.start,
        f.control,
        f.effect,
        f.args[0],
        f.args[1],
        f.args[2],
    ];
    let dead: Vec<_> = f.graph.ids().filter(|node| !kept.contains(node)).collect();
    for node in dead {
        f.graph.remove_node(node);
    }
    let zero = f.graph.add_constant(0, f.word);
    let one = f.graph.add_constant(1, f.word);
    let limit = f.graph.add_constant(4, f.word);
    let existing = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[zero, one],
        0,
        ValueType::Rust(f.word),
    );
    let observe = f.graph.add_node(
        Opcode::RequireDefined,
        f.control,
        &[f.effect, existing],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = projection(&mut f.graph, observe, 0, ValueType::Control);
    let effect = projection(&mut f.graph, observe, 1, ValueType::Effect);
    let entry = f
        .graph
        .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let phi = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, zero],
        0,
        ValueType::Rust(f.word),
    );
    let compare = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[phi, limit],
        ComparePredicate::UnsignedLess as u32,
        ValueType::Rust(f.boolean),
    );
    let branch = f.graph.add_node(
        Opcode::If,
        header,
        &[compare],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let body = projection(&mut f.graph, branch, 0, ValueType::Control);
    let exit = projection(&mut f.graph, branch, 1, ValueType::Control);
    let next = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[phi, one],
        0,
        ValueType::Rust(f.word),
    );
    let latch = f
        .graph
        .add_node(Opcode::Jump, body, &[], 0, ValueType::Control);
    f.graph.set_inputs(header, &[entry, latch]);
    f.graph.set_inputs(phi, &[header, zero, next]);
    f.graph
        .add_node(Opcode::Return, exit, &[effect, phi], 0, ValueType::Control);
    no_change(f, Limits::default());
}

#[test]
fn budget_rejection_is_atomic_and_r_or_l_graphs_are_untouched() {
    for limits in [
        Limits {
            plans: 0,
            ..Limits::default()
        },
        Limits {
            nodes: 1,
            ..Limits::default()
        },
        Limits {
            analysis: 1,
            ..Limits::default()
        },
    ] {
        assert!(
            no_change(Fixture::new(Opcode::Add, [true, false], false), limits).budget_bailouts > 0
        );
    }
    for level in [GraphLevel::Rust, GraphLevel::Layout] {
        let mut f = Fixture::new(Opcode::Add, [true, false], false);
        f.graph.level = level;
        let before = f.graph.dump();
        assert_eq!(run(&mut f.graph), PreStats::default());
        assert_eq!(before, f.graph.dump());
    }
}

#[test]
fn bool_and_char_pre_keep_exact_result_types_and_poison_dependencies() {
    for shape in [TypeShape::Bool, TypeShape::Char] {
        let is_bool = shape == TypeShape::Bool;
        let mut f = Fixture::new(
            if is_bool {
                Opcode::BitXor
            } else {
                Opcode::BitAnd
            },
            [true, false],
            false,
        );
        let old_word = f.word;
        let size = if is_bool { 1 } else { 4 };
        f.word = f
            .graph
            .oracle
            .intern(shape, Some(SizeAlign { size, align: size }));
        let nodes: Vec<_> = f
            .graph
            .ids()
            .filter(|&node| f.graph.rust_type(node) == Some(old_word))
            .collect();
        for node in nodes {
            f.graph.set_value_type(node, ValueType::Rust(f.word));
        }
        f.graph.set_value_type(
            f.graph.start,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(f.boolean),
                ValueType::Rust(f.word),
                ValueType::Rust(f.word),
            ]),
        );
        let old_delta = f.delta;
        f.delta = f.graph.add_constant(1, f.word);
        for node in [f.existing[0], f.existing[1], f.result] {
            f.graph.set_input(node, 1, f.delta);
        }
        f.graph.remove_node(old_delta);
        f.verify();
        let before = f.graph.clone();
        assert_eq!(run(&mut f.graph).expressions_replaced, 1);
        assert_eq!(f.graph.rust_type(f.graph.input(f.ret, 1)), Some(f.word));
        f.verify();
        let values = if is_bool {
            vec![0, 1]
        } else {
            vec![0, 1, 127, 255, 0xd7ff, 0xe000, 0x10ffff]
        };
        for side in [0, 1] {
            for &value in &values {
                let args = [
                    Value::Scalar(side),
                    Value::Scalar(value),
                    Value::Scalar(value),
                ];
                assert_eq!(execute(&before, &args), execute(&f.graph, &args));
            }
            let args = [Value::Scalar(side), Value::Poison, Value::Scalar(1)];
            assert_eq!(
                execute(&before, &args).is_err(),
                execute(&f.graph, &args).is_err()
            );
        }
    }
}
