//! Fixed values sharing a control anchor follow dependencies, not arena IDs.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::lower_function;

fn reverse_dependencies(opcode: Opcode, pure_bridge: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let ty = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: matches!(opcode, Opcode::DivSigned | Opcode::RemSigned),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut graph = FunctionGraph::new(
        format!("anchored_{}_{}", opcode.name(), u8::from(pure_bridge)),
        FunctionAbi {
            args: vec![],
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
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
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
    let dividend = graph.add_constant(40, ty);
    let divisor = graph.add_constant(5, ty);
    let consumer = graph.add_node(
        opcode,
        control,
        &[dividend, divisor],
        0,
        ValueType::Rust(ty),
    );
    let one = graph.add_constant(1, ty);
    let producer_op = if matches!(opcode, Opcode::DivSigned | Opcode::RemSigned) {
        Opcode::DivSigned
    } else {
        Opcode::DivUnsigned
    };
    let first = graph.add_node(
        producer_op,
        control,
        &[dividend, one],
        0,
        ValueType::Rust(ty),
    );
    let second = graph.add_node(
        producer_op,
        control,
        &[dividend, one],
        0,
        ValueType::Rust(ty),
    );
    graph.set_input(first, 0, second);
    let operand = if pure_bridge {
        let two = graph.add_constant(2, ty);
        graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[first, two],
            0,
            ValueType::Rust(ty),
        )
    } else {
        first
    };
    graph.set_input(consumer, 0, operand);
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, consumer],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

#[test]
fn all_anchored_arithmetic_follows_later_divisions_through_pure_values() {
    for opcode in [
        Opcode::DivSigned,
        Opcode::DivUnsigned,
        Opcode::RemSigned,
        Opcode::RemUnsigned,
    ] {
        for bridge in [false, true] {
            let graph = reverse_dependencies(opcode, bridge);
            let machine =
                lower_function(&graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
            let input = if bridge { 42 } else { 40 };
            let expected = if matches!(opcode, Opcode::DivSigned | Opcode::DivUnsigned) {
                input / 5
            } else {
                input % 5
            };
            assert_eq!(
                kfrc_x86::machine::eval::evaluate(&machine, &[]).unwrap(),
                expected
            );
            assert_eq!(
                Interpreter::new(&graph)
                    .run(&[], &mut |_: &str, _: &[u64]| Ok(0))
                    .unwrap(),
                Outcome::Return(expected as u64)
            );
        }
    }
}
