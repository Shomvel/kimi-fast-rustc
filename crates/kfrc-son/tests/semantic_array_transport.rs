//! Whole-array transport stays compact while observations and Phis keep leaves.
use kfrc_body::{
    AggregateLayout, EnumLayout, FieldLayout, FunctionAbi, IntegerWidth, PassMode, RecordedTypes,
    SizeAlign, TagEncoding, TypeOracle, TypeShape,
};
use kfrc_son::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType, lower_layout, verify_graph};

fn fixture(observe: bool, phi: bool, length: u64) -> FunctionGraph {
    let mut oracle = RecordedTypes::new();
    let byte = oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let array = oracle.intern(
        TypeShape::Array {
            element: byte,
            length,
        },
        Some(SizeAlign {
            size: length,
            align: 1,
        }),
    );
    let mut graph = FunctionGraph::new(
        "array_transport",
        FunctionAbi {
            args: vec![PassMode::Indirect],
            ret: if observe {
                PassMode::Direct
            } else {
                PassMode::Indirect
            },
            c_variadic: false,
        },
        oracle,
    );
    graph.level = GraphLevel::Specialized;
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(array),
        ]),
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
    let mut value = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        2,
        ValueType::Rust(array),
    );
    let control = if phi {
        let jump = graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
        let region = graph.add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
        value = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, value],
            0,
            ValueType::Rust(array),
        );
        region
    } else {
        control
    };
    if observe {
        value = graph.add_node(
            Opcode::Field,
            NodeId::NONE,
            &[value],
            7,
            ValueType::Rust(byte),
        );
    }
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    graph
}

#[test]
fn whole_dense_array_uses_block_snapshot_instead_of_scalar_leaves() {
    let mut graph = fixture(false, false, 256);
    let stats = lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(stats.whole_value_transports, 1);
    assert_eq!(stats.transported_arrays, 0);
    assert_eq!(stats.field_loads, 0);
    assert_eq!(stats.field_stores, 0);
    assert!(graph.len() < 40, "{}", graph.dump());
}

#[test]
fn element_observation_preserves_scalar_snapshot_representation() {
    let mut graph = fixture(true, false, 256);
    let stats = lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(stats.transported_arrays, 0);
    assert_eq!(stats.field_loads, 256);
}

#[test]
fn phi_transport_does_not_carry_reusable_snapshot_addresses() {
    let mut graph = fixture(false, true, 256);
    let stats = lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(stats.transported_arrays, 0);
}

#[test]
fn whole_array_can_feed_an_owned_enum_phi() {
    let mut graph = fixture(false, false, 64);
    let ret = graph
        .ids()
        .find(|&node| graph.op(node) == Opcode::Return)
        .unwrap();
    let array_value = graph.input(ret, 1);
    let array = graph.rust_type(array_value).unwrap();
    let &TypeShape::Array { element: byte, .. } = graph.oracle.shape(array) else {
        unreachable!();
    };
    let enumeration = graph.oracle.intern(
        TypeShape::Adt {
            name: "ArrayOrEmpty".into(),
        },
        Some(SizeAlign { size: 65, align: 1 }),
    );
    graph.oracle.record_aggregate(
        enumeration,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![FieldLayout {
                    ty: array,
                    offset: 1,
                }],
                vec![],
            ],
        },
    );
    let discriminants = vec![0, 1];
    graph
        .oracle
        .record_semantic_enum(enumeration, discriminants.clone());
    graph.oracle.enums.insert(
        enumeration,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: byte,
                offset: 0,
            }),
            encoding: TagEncoding::Direct { discriminants },
        },
    );
    let variant = graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[array_value],
        0,
        ValueType::Rust(enumeration),
    );
    let entry = graph.add_node(Opcode::Jump, graph.control(ret), &[], 0, ValueType::Control);
    let region = graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let result = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, variant],
        0,
        ValueType::Rust(enumeration),
    );
    graph.set_control(ret, region);
    graph.set_input(ret, 1, result);
    verify_graph(&mut graph).unwrap();

    let stats = lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(stats.owned_transport_phis, 1);
    assert_eq!(stats.transported_arrays, 1);
    assert_eq!(stats.field_loads, 0, "the payload stays a whole array");
}

#[test]
fn small_arrays_share_whole_value_transport() {
    let mut graph = fixture(false, false, 8);
    let stats = lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(stats.transported_arrays, 0);
    assert_eq!(stats.whole_value_transports, 1);
    assert_eq!(stats.field_loads, 0);
    assert_eq!(stats.field_stores, 0);
}
