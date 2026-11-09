//! Behavioral regression for the shared typed/raw edge-snapshot scheduler.
use super::*;
use crate::{InterpretError, Interpreter, Outcome, verify_graph};
use kfrc_body::{AggregateLayout, FieldLayout, FunctionAbi, PassMode, RecordedTypes};

#[test]
fn typed_snapshot_captures_old_raw_bytes_before_same_edge_overwrite() {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let pair = types.intern(
        TypeShape::Tuple(vec![word, word]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        pair,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: word,
                    offset: 0,
                },
                FieldLayout {
                    ty: word,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    let raw = types.intern(
        TypeShape::Adt {
            name: "MaybeUninit<Pair>".into(),
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_maybe_uninit(raw, pair);
    let wrapper = types.intern(
        TypeShape::Tuple(vec![raw]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        wrapper,
        AggregateLayout {
            fields: vec![FieldLayout { ty: raw, offset: 0 }],
            variants: vec![],
        },
    );
    let mut graph = FunctionGraph::new(
        "mixed_edge_snapshots",
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
    let mut control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let mut effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let mut lower = LayoutLowering {
        projections: projections::ProjectionIndex::new(&graph),
        graph: &mut graph,
        values: HashMap::default(),
        demands: HashMap::default(),
        owned_transport: HashSet::default(),
        array_transport: HashSet::default(),
        boundary_transport: HashMap::default(),
        owned_phis: Vec::new(),
        stats: LayoutStats::default(),
        retired: Vec::new(),
        partition: 0,
        tag_only_loads: HashSet::default(),
        options: LayoutOptions::default(),
        encoded_values: HashSet::default(),
        variant_tags: HashMap::default(),
        unreachable_switch_edges: HashMap::default(),
        opaque_phis: Vec::new(),
        opaque_phi_regions: HashMap::default(),
        lane_plans: HashMap::default(),
    };
    // Only the first word is initialized. Whole representation copies must not
    // observe or initialize the second word while transporting this snapshot.
    let old = lower.opaque_slot(raw).unwrap();
    let replacement = lower.opaque_slot(raw).unwrap();
    let captured = lower.opaque_slot(wrapper).unwrap();
    let eleven = lower.graph.add_constant(11, word);
    let twenty_two = lower.graph.add_constant(22, word);
    let old_first = lower.field_address(old, 0, word);
    let replacement_first = lower.field_address(replacement, 0, word);
    lower.store(old_first, eleven, &mut control, &mut effect);
    lower.store(replacement_first, twenty_two, &mut control, &mut effect);
    let jump = lower
        .graph
        .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
    let region = lower
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
    // A typed constructor's raw field is captured before the raw Phi writes
    // its next snapshot into the same storage. Test the shared scheduling
    // contract directly; selection's CFG restrictions are tested separately.
    lower.owned_phis.push(OwnedPhiInit {
        region,
        slot: 1,
        source: Value {
            ty: wrapper,
            parts: Parts::Fields(vec![Value {
                ty: raw,
                parts: Parts::OpaqueStorage(old),
            }]),
        },
        destination: captured,
    });
    lower.opaque_phis.push(maybe_uninit::OpaquePhiCopy {
        region,
        slot: 1,
        source: replacement,
        destination: old,
        ty: raw,
    });
    lower.finish_snapshot_phis().unwrap();
    control = region;
    effect = lower.opaque_effect_at(region, &mut HashMap::default()).unwrap();
    let captured_first = lower.field_address(captured, 0, word);
    let before = lower.load(captured_first, word, &mut control, &mut effect);
    let after = lower.load(old_first, word, &mut control, &mut effect);
    let hundred = lower.graph.add_constant(100, word);
    let scaled = lower.graph.add_node(
        Opcode::Mul,
        NodeId::NONE,
        &[before, hundred],
        0,
        ValueType::Rust(word),
    );
    let result = lower.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[scaled, after],
        0,
        ValueType::Rust(word),
    );
    lower.graph.add_node(
        Opcode::Return,
        control,
        &[effect, result],
        0,
        ValueType::Control,
    );
    verify_graph(lower.graph).unwrap();
    assert_eq!(
        Interpreter::new(lower.graph)
            .run(&[], &mut |name: &str, _: &[u64]| Err(
                InterpretError::UnknownCallee(name.into())
            ))
            .unwrap(),
        Outcome::Return(1122)
    );

    // A typed observation of the untouched second word must still fail after
    // the raw transport; copying bytes must not turn uninitialized storage
    // into a defined scalar.
    let ret = lower
        .graph
        .ids()
        .find(|&node| lower.graph.op(node) == Opcode::Return)
        .unwrap();
    let second_address = lower.field_address(captured, 8, word);
    let second = lower.load(second_address, word, &mut control, &mut effect);
    lower.graph.set_control(ret, control);
    lower.graph.set_input(ret, 0, effect);
    lower.graph.set_input(ret, 1, second);
    verify_graph(lower.graph).unwrap();
    assert!(matches!(
        Interpreter::new(lower.graph).run(&[], &mut |name: &str, _: &[u64]| Err(
            InterpretError::UnknownCallee(name.into())
        )),
        Err(InterpretError::PoisonObserved(_))
    ));
}
