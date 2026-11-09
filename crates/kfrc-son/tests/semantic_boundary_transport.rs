//! Whole call-result subfields keep a private representation snapshot.
use kfrc_body::{
    AggregateLayout, EnumLayout, FieldLayout, FunctionAbi, IntegerWidth, PassMode, RecordedTypes,
    SizeAlign, TagEncoding, TyId, TypeShape,
};
use kfrc_son::{
    CallTarget, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode, Outcome,
    ValueType, lower_layout, verify_graph,
};

use kfrc_son::interpret::CallHandler;

fn enumeration(
    types: &mut RecordedTypes,
    name: &str,
    size: u64,
    tag: TyId,
    variants: Vec<Vec<FieldLayout>>,
) -> TyId {
    let ty = types.intern(
        TypeShape::Adt { name: name.into() },
        Some(SizeAlign { size, align: 8 }),
    );
    let discriminants = (0..variants.len() as u128).collect::<Vec<_>>();
    types.record_aggregate(
        ty,
        AggregateLayout {
            fields: vec![],
            variants,
        },
    );
    types.record_semantic_enum(ty, discriminants.clone());
    types.enums.insert(
        ty,
        EnumLayout {
            tag: Some(FieldLayout { ty: tag, offset: 0 }),
            encoding: TagEncoding::Direct { discriminants },
        },
    );
    ty
}

// Three cases: outer Ok, nested fieldless Err, nested Err with u32 payload.
// The nested enum has physical padding; the typed result ignores that padding.
fn fixture(observe_duplicate: bool, cyclic: bool) -> FunctionGraph {
    fixture_with_raw_field(observe_duplicate, cyclic, false)
}

fn fixture_with_raw_field(observe_duplicate: bool, cyclic: bool, raw_field: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let byte = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let small = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W32,
            signed: false,
        },
        Some(SizeAlign { size: 4, align: 4 }),
    );
    let small = if raw_field {
        let raw = types.intern(
            TypeShape::Adt {
                name: "MaybeUninitWord".into(),
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        types.record_union(
            raw,
            vec![FieldLayout {
                ty: small,
                offset: 0,
            }],
        );
        types.record_maybe_uninit(raw, small);
        raw
    } else {
        small
    };
    let nested = enumeration(
        &mut types,
        "Nested",
        8,
        byte,
        vec![
            vec![],
            vec![FieldLayout {
                ty: small,
                offset: 4,
            }],
        ],
    );
    let outer = enumeration(
        &mut types,
        "Outer",
        16,
        word,
        vec![
            vec![FieldLayout {
                ty: word,
                offset: 8,
            }],
            vec![FieldLayout {
                ty: nested,
                offset: 8,
            }],
        ],
    );
    types.record_abi_pair(
        outer,
        [
            FieldLayout {
                ty: word,
                offset: 0,
            },
            FieldLayout {
                ty: word,
                offset: 8,
            },
        ],
    );
    let mut g = FunctionGraph::new(
        "boundary_transport",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Pair,
            c_variadic: false,
        },
        types,
    );
    g.level = GraphLevel::Specialized;
    let start = g.add_node(
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
    g.start = start;
    let mut control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let effect = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let argument = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        2,
        ValueType::Rust(word),
    );
    // A control cycle anywhere in the function disables this representation,
    // even when the call's data graph itself is acyclic.
    if cyclic {
        let entry = g.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
        let back = g.add_node(Opcode::Jump, NodeId::NONE, &[], 0, ValueType::Control);
        let region = g.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[entry, back],
            0,
            ValueType::Control,
        );
        let boolean = g
            .oracle
            .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let condition = g.add_constant(0, boolean);
        let branch = g.add_node(
            Opcode::If,
            region,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        let repeat = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            0,
            ValueType::Control,
        );
        control = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            1,
            ValueType::Control,
        );
        g.set_control(back, repeat);
    }
    let target = g.add_callee(CallTarget {
        symbol: "source".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Pair,
            c_variadic: false,
        },
    });
    let call = g.add_node(
        Opcode::Call,
        control,
        &[effect, argument],
        target,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(outer),
        ]),
    );
    let control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        0,
        ValueType::Control,
    );
    let effect = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        1,
        ValueType::Effect,
    );
    let root = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        2,
        ValueType::Rust(outer),
    );
    g.variant_switches.push(vec![0]);
    let branch = g.add_node(
        Opcode::VariantSwitch,
        control,
        &[root],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    for (variant, ty) in [(0, word), (1, nested)] {
        let mut control = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            variant,
            ValueType::Control,
        );
        let witness = g.add_node(
            Opcode::VariantWitness,
            control,
            &[root, control],
            variant,
            ValueType::Witness,
        );
        let field_index = g.variant_fields.len() as u32;
        g.variant_fields.push((variant, 0));
        let field = g.add_node(
            Opcode::VariantField,
            NodeId::NONE,
            &[root, witness],
            field_index,
            ValueType::Rust(ty),
        );
        let mut effect = effect;
        if variant == 1 && observe_duplicate {
            let duplicate = g.add_node(
                Opcode::VariantField,
                NodeId::NONE,
                &[root, witness],
                field_index,
                ValueType::Rust(ty),
            );
            let tag = g.add_node(
                Opcode::Discriminant,
                NodeId::NONE,
                &[duplicate],
                0,
                ValueType::Rust(word),
            );
            let target = g.add_callee(CallTarget {
                symbol: "observe".into(),
                abi: FunctionAbi {
                    args: vec![PassMode::Direct],
                    ret: PassMode::Ignore,
                    c_variadic: false,
                },
            });
            let unit = g
                .oracle
                .intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
            let call = g.add_node(
                Opcode::Call,
                control,
                &[effect, tag],
                target,
                ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(unit),
                ]),
            );
            control = g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[call],
                0,
                ValueType::Control,
            );
            effect = g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[call],
                1,
                ValueType::Effect,
            );
        }
        let result = g.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[field],
            variant,
            ValueType::Rust(outer),
        );
        g.add_node(
            Opcode::Return,
            control,
            &[effect, result],
            0,
            ValueType::Control,
        );
    }
    verify_graph(&mut g).unwrap();
    g
}

struct Calls;
impl CallHandler for Calls {
    fn call(&mut self, symbol: &str, _: &[u64]) -> Result<u64, InterpretError> {
        assert_eq!(symbol, "observe");
        Ok(0)
    }
    fn call_pair(&mut self, symbol: &str, args: &[u64]) -> Result<(u64, u64), InterpretError> {
        assert_eq!(symbol, "source");
        Ok(match args[0] {
            0 => (0, 123),
            1 => (1, 0),
            _ => (1, (987u64 << 32) | 1),
        })
    }
}

fn check(mut graph: FunctionGraph, expected_captures: usize) {
    let stats = lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(
        stats.transported_boundary_fields,
        expected_captures,
        "{}",
        graph.dump()
    );
    for (input, expected) in [
        (0, Outcome::ReturnPair(0, 123)),
        (1, Outcome::ReturnPair(1, 0)),
        (2, Outcome::ReturnPair(1, (987u64 << 32) | 1)),
    ] {
        assert_eq!(
            Interpreter::new(&graph).run(&[input], &mut Calls).unwrap(),
            expected
        );
    }
}

#[test]
fn direct_call_subfield_is_captured_once_and_nested_variant_bytes_survive() {
    check(fixture(false, false), 1);
}
#[test]
fn any_observed_duplicate_extractor_disables_the_entire_field_certificate() {
    check(fixture(true, false), 0);
}
#[test]
fn whole_function_control_cycle_disables_boundary_snapshot_transport() {
    check(fixture(false, true), 0);
}

#[test]
fn nested_maybe_uninit_subtree_keeps_existing_raw_snapshot_semantics() {
    check(fixture_with_raw_field(false, false, true), 0);
}
