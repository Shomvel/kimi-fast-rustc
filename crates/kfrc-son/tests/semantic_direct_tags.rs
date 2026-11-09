//! Direct tags survive construction, Phi merges and memory without an ordinal round trip.
use kfrc_body::{
    AggregateLayout, EnumLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode,
    RecordedTypes, SizeAlign, TagEncoding, TypeShape,
};
use kfrc_son::{
    ComparePredicate, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode,
    Outcome, SemanticOutcome, Value, ValueType, lower_layout, verify_graph,
};

fn fixture(
    discriminants: &[u128],
    width: IntegerWidth,
    signed: bool,
    query: Opcode,
) -> FunctionGraph {
    let size = match width {
        IntegerWidth::W8 => 1,
        IntegerWidth::W32 => 4,
        IntegerWidth::W64 => 8,
        _ => unreachable!(),
    };
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let tag = types.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign { size, align: size }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "SparseTags".into(),
        },
        Some(SizeAlign { size, align: size }),
    );
    types.record_aggregate(
        enumeration,
        AggregateLayout {
            fields: vec![],
            variants: vec![vec![]; discriminants.len()],
        },
    );
    types.record_semantic_enum(enumeration, discriminants.to_vec());
    types.enums.insert(
        enumeration,
        EnumLayout {
            tag: Some(FieldLayout { ty: tag, offset: 0 }),
            encoding: TagEncoding::Direct {
                discriminants: discriminants.to_vec(),
            },
        },
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: enumeration,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut graph = FunctionGraph::new(
        "direct_tags",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
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
            ValueType::Rust(word),
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
    let input = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        2,
        ValueType::Rust(word),
    );
    let zero = graph.add_constant(0, word);
    let condition = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[input, zero],
        ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let branch = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let edges = [0, 1].map(|index| {
        let edge = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            index,
            ValueType::Control,
        );
        graph.add_node(Opcode::Jump, edge, &[], 0, ValueType::Control)
    });
    let region = graph.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
    let first = graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(enumeration),
    );
    let last = graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[],
        discriminants.len() as u32 - 1,
        ValueType::Rust(enumeration),
    );
    let merged = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, first, last],
        0,
        ValueType::Rust(enumeration),
    );
    let slot = graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    let store = graph.add_node(
        Opcode::StoreValue,
        region,
        &[effect, slot, merged],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[store],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[store],
        1,
        ValueType::Effect,
    );
    let load = graph.add_node(
        Opcode::LoadValue,
        control,
        &[effect, slot],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(enumeration),
        ]),
    );
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        1,
        ValueType::Effect,
    );
    let loaded = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        2,
        ValueType::Rust(enumeration),
    );
    let result = graph.add_node(query, NodeId::NONE, &[loaded], 0, ValueType::Rust(word));
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, result],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    graph
}

fn compare(mut graph: FunctionGraph, expected: [u64; 2]) -> FunctionGraph {
    for (input, expected) in expected.iter().enumerate() {
        assert_eq!(
            Interpreter::new(&graph)
                .run_values(
                    &[Value::Scalar(input as u64)],
                    &mut |name: &str, _: &[Value]| Err(InterpretError::UnknownCallee(name.into()))
                )
                .unwrap(),
            SemanticOutcome::Return(Value::Scalar(*expected))
        );
    }
    lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    for (input, expected) in expected.iter().enumerate() {
        assert_eq!(
            Interpreter::new(&graph)
                .run(&[input as u64], &mut |name: &str, _: &[u64]| Err(
                    InterpretError::UnknownCallee(name.into())
                ))
                .unwrap(),
            Outcome::Return(*expected)
        );
    }
    graph
}

#[test]
fn direct_discriminants_have_constant_cost_through_phi_and_memory() {
    let mut counts = None;
    for count in [2, 16, 64] {
        let tags: Vec<_> = (0..count).map(|index| 1u128 << index).collect();
        let graph = compare(
            fixture(&tags, IntegerWidth::W64, false, Opcode::Discriminant),
            [1, tags[count - 1] as u64],
        );
        let current: Vec<_> = [
            Opcode::Compare,
            Opcode::If,
            Opcode::Load,
            Opcode::Store,
            Opcode::Phi,
            Opcode::BitAnd,
            Opcode::BitOr,
        ]
        .map(|op| graph.ids().filter(|&node| graph.op(node) == op).count())
        .into();
        assert_eq!(current[0], 1, "only the source selection compares values");
        assert_eq!(current[5..], [0, 0], "no arithmetic tag-selection chains");
        assert_eq!(current, *counts.get_or_insert(current.clone()));
    }
}

#[test]
fn narrow_signed_and_unsigned_tags_preserve_discriminants_and_ordinals() {
    for (tags, width, signed) in [
        (vec![3, 127, 255], IntegerWidth::W8, false),
        (
            vec![(-128i128) as u128, 7, (-1i128) as u128],
            IntegerWidth::W8,
            true,
        ),
        (
            vec![(i32::MIN as i128) as u128, 17, i32::MAX as u128],
            IntegerWidth::W32,
            true,
        ),
    ] {
        compare(
            fixture(&tags, width, signed, Opcode::Discriminant),
            [tags[0] as u64, tags[2] as u64],
        );
        compare(fixture(&tags, width, signed, Opcode::VariantOf), [0, 2]);
    }
}

#[test]
fn known_direct_variant_index_remains_constant() {
    let mut graph = fixture(&[3, 17, 255], IntegerWidth::W8, false, Opcode::VariantOf);
    let query = graph
        .ids()
        .find(|&node| graph.op(node) == Opcode::VariantOf)
        .unwrap();
    let ty = graph.rust_type(graph.input(query, 0)).unwrap();
    let known = graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[],
        2,
        ValueType::Rust(ty),
    );
    graph.set_input(query, 0, known);
    verify_graph(&mut graph).unwrap();
    let graph = compare(graph, [2, 2]);
    assert_eq!(
        graph
            .ids()
            .filter(|&node| graph.op(node) == Opcode::Compare)
            .count(),
        1
    );
}
