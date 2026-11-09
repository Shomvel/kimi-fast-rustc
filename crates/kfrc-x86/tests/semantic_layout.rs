//! Semantic snapshots and loop Phi values survive the actual layout seam.
#[path = "../../kfrc-son/tests/support/refinement.rs"]
mod refinement;

use kfrc_body::*;
use kfrc_son::{
    FunctionGraph, GraphLevel, LayoutError, Opcode, ValueType, build_graph, lower_layout,
    verify_graph,
};
use kfrc_x86::lower_function;

fn local(b: &mut BodyBuilder, ty: TyId) -> LocalId {
    b.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}
fn copy(local: LocalId) -> Operand {
    Operand::Copy(Place::local(local))
}
fn assign(b: &mut BodyBuilder, place: Place, value: Rvalue) {
    b.push_statement(StatementKind::Assign(place, value));
}
fn field(b: &mut BodyBuilder, local: LocalId, index: u32, ty: TyId) -> Place {
    Place {
        local,
        projection: b.add_projection(vec![ProjectionElem::Field(FieldIndex(index), ty)]),
    }
}
fn constant(b: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(b.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(bits),
    }))
}
fn types() -> (RecordedTypes, TyId, TyId, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let pair = types.intern(
        TypeShape::Tuple(vec![word, word]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    let fields = [
        FieldLayout {
            ty: word,
            offset: 0,
        },
        FieldLayout {
            ty: word,
            offset: 8,
        },
    ];
    types.record_aggregate(
        pair,
        AggregateLayout {
            fields: fields.to_vec(),
            variants: vec![],
        },
    );
    types.record_abi_pair(pair, fields);
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    (types, word, boolean, pair, unit)
}
fn builder(name: &str, types: RecordedTypes, args: Vec<PassMode>, ret: PassMode) -> BodyBuilder {
    BodyBuilder::new(
        name,
        FunctionAbi {
            args,
            ret,
            c_variadic: false,
        },
        types,
    )
}
fn lower(body: &Body) -> FunctionGraph {
    let mut graph = build_graph(body).unwrap();
    assert!(graph.stack_slots.is_empty());
    verify_graph(&mut graph).unwrap_or_else(|e| panic!("{e}\n{}", graph.dump()));
    assert!(
        lower_function(&graph).is_err(),
        "S cannot bypass the layout seam"
    );
    lower_layout(&mut graph).unwrap_or_else(|e| panic!("{e}\n{}", graph.dump()));
    assert_eq!(graph.level, GraphLevel::Layout);
    assert!(!graph.ids().any(|node| graph.op(node).is_semantic_value()));
    verify_graph(&mut graph).unwrap_or_else(|e| panic!("{e}\n{}", graph.dump()));
    lower_function(&graph).unwrap_or_else(|e| panic!("{e}\n{}", graph.dump()));
    graph
}
fn swap_pair() -> Body {
    let (types, word, _, pair, _) = types();
    let mut b = builder("semantic_swap", types, vec![PassMode::Pair], PassMode::Pair);
    let ret = local(&mut b, pair);
    let arg = local(&mut b, pair);
    b.set_arg_count(1);
    let a = field(&mut b, arg, 0, word);
    let z = field(&mut b, arg, 1, word);
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::Aggregate(
            AggregateKind::Tuple,
            vec![Operand::Copy(z), Operand::Copy(a)],
        ),
    );
    b.finish_block(Terminator::Return, false);
    b.finish()
}
fn call_pair() -> Body {
    let (types, word, _, pair, unit) = types();
    let mut b = builder(
        "semantic_call",
        types,
        vec![PassMode::Pair],
        PassMode::Direct,
    );
    let ret = local(&mut b, word);
    let arg = local(&mut b, pair);
    b.set_arg_count(1);
    let result = local(&mut b, pair);
    let callee = Operand::Constant(b.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "semantic_swap".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Pair],
                ret: PassMode::Pair,
                c_variadic: false,
            },
        }),
    }));
    b.finish_block(
        Terminator::Call {
            callee,
            args: vec![copy(arg)],
            destination: Place::local(result),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    let a = field(&mut b, result, 0, word);
    let z = field(&mut b, result, 1, word);
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::BinaryOp(BinaryOp::Sub, Operand::Copy(a), Operand::Copy(z)),
    );
    b.finish_block(Terminator::Return, false);
    b.finish()
}

fn snapshot_across_aliasing_call() -> Body {
    let (mut types, word, _, _, unit) = types();
    let triple = types.intern(
        TypeShape::Tuple(vec![word, word, word]),
        Some(SizeAlign { size: 24, align: 8 }),
    );
    types.record_aggregate(
        triple,
        AggregateLayout {
            fields: (0..3)
                .map(|index| FieldLayout {
                    ty: word,
                    offset: index * 8,
                })
                .collect(),
            variants: vec![],
        },
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut b = builder(
        "semantic_snapshot",
        types,
        vec![PassMode::Indirect, PassMode::Direct],
        PassMode::Indirect,
    );
    let ret = local(&mut b, triple);
    let input = local(&mut b, triple);
    let alias = local(&mut b, pointer);
    b.set_arg_count(2);
    let ignored = local(&mut b, unit);
    let callee = Operand::Constant(b.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "mutate_first".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
        }),
    }));
    b.finish_block(
        Terminator::Call {
            callee,
            args: vec![copy(alias)],
            destination: Place::local(ignored),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    assign(&mut b, Place::local(ret), Rvalue::Use(copy(input)));
    b.finish_block(Terminator::Return, false);
    b.finish()
}
fn fibonacci() -> Body {
    let (types, word, boolean, pair, _) = types();
    let mut b = builder(
        "semantic_fibonacci",
        types,
        vec![PassMode::Direct],
        PassMode::Direct,
    );
    let ret = local(&mut b, word);
    let n = local(&mut b, word);
    b.set_arg_count(1);
    let state = local(&mut b, pair);
    let index = local(&mut b, word);
    let next = local(&mut b, word);
    let condition = local(&mut b, boolean);
    let zero = constant(&mut b, word, 0);
    let one = constant(&mut b, word, 1);
    assign(
        &mut b,
        Place::local(state),
        Rvalue::Aggregate(AggregateKind::Tuple, vec![zero, one]),
    );
    assign(&mut b, Place::local(index), Rvalue::Use(zero));
    b.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(
        &mut b,
        Place::local(condition),
        Rvalue::BinaryOp(BinaryOp::Lt, copy(index), copy(n)),
    );
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(condition),
            targets: SwitchTargets {
                cases: vec![(1, BlockId(2))],
                otherwise: BlockId(3),
            },
        },
        false,
    );
    let a = field(&mut b, state, 0, word);
    let z = field(&mut b, state, 1, word);
    assign(
        &mut b,
        Place::local(next),
        Rvalue::BinaryOp(BinaryOp::Add, Operand::Copy(a), Operand::Copy(z)),
    );
    assign(
        &mut b,
        Place::local(state),
        Rvalue::Aggregate(AggregateKind::Tuple, vec![Operand::Copy(z), copy(next)]),
    );
    assign(
        &mut b,
        Place::local(index),
        Rvalue::BinaryOp(BinaryOp::Add, copy(index), one),
    );
    b.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(&mut b, Place::local(ret), Rvalue::Use(Operand::Copy(a)));
    b.finish_block(Terminator::Return, false);
    b.finish()
}

fn enum_functions() -> Vec<Body> {
    let (mut types, word, boolean, _, _) = types();
    let byte = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let choice = types.intern(
        TypeShape::Adt {
            name: "DynamicChoice".into(),
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        choice,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![],
                vec![FieldLayout {
                    ty: word,
                    offset: 8,
                }],
            ],
        },
    );
    types.record_semantic_enum(choice, vec![3, 9]);
    types.enums.insert(
        choice,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: byte,
                offset: 0,
            }),
            encoding: TagEncoding::Direct {
                discriminants: vec![3, 9],
            },
        },
    );
    types.record_abi_pair(
        choice,
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
    let mut identity = builder(
        "semantic_enum_identity",
        types.clone(),
        vec![PassMode::Pair],
        PassMode::Pair,
    );
    let ret = local(&mut identity, choice);
    let input = local(&mut identity, choice);
    identity.set_arg_count(1);
    assign(&mut identity, Place::local(ret), Rvalue::Use(copy(input)));
    identity.finish_block(Terminator::Return, false);
    let mut select = builder(
        "semantic_enum_select",
        types.clone(),
        vec![PassMode::Direct, PassMode::Pair, PassMode::Pair],
        PassMode::Pair,
    );
    let ret = local(&mut select, choice);
    let condition = local(&mut select, boolean);
    let yes = local(&mut select, choice);
    let no = local(&mut select, choice);
    select.set_arg_count(3);
    select.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(condition),
            targets: SwitchTargets {
                cases: vec![(1, BlockId(1))],
                otherwise: BlockId(2),
            },
        },
        false,
    );
    assign(&mut select, Place::local(ret), Rvalue::Use(copy(yes)));
    select.finish_block(Terminator::Goto(BlockId(3)), false);
    assign(&mut select, Place::local(ret), Rvalue::Use(copy(no)));
    select.finish_block(Terminator::Goto(BlockId(3)), false);
    select.finish_block(Terminator::Return, false);
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let niche = types.intern(
        TypeShape::Adt {
            name: "NichePointer".into(),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    types.record_aggregate(
        niche,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![],
                vec![FieldLayout {
                    ty: pointer,
                    offset: 0,
                }],
            ],
        },
    );
    types.record_semantic_enum(niche, vec![0, 1]);
    types.enums.insert(
        niche,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: pointer,
                offset: 0,
            }),
            encoding: TagEncoding::Niche {
                untagged_variant: 1,
                niche_variants: 0..=0,
                niche_start: 0,
            },
        },
    );
    let mut option = builder(
        "semantic_niche_identity",
        types,
        vec![PassMode::Direct],
        PassMode::Direct,
    );
    let ret = local(&mut option, niche);
    let input = local(&mut option, niche);
    option.set_arg_count(1);
    assign(&mut option, Place::local(ret), Rvalue::Use(copy(input)));
    option.finish_block(Terminator::Return, false);
    vec![identity.finish(), select.finish(), option.finish()]
}

#[test]
fn dynamic_enum_niche_and_phi_lower_with_guarded_payload_reads() {
    for body in enum_functions() {
        let graph = lower(&body);
        assert!(graph.ids().any(|node| graph.op(node) == Opcode::If));
        assert!(graph.ids().any(|node| graph.op(node) == Opcode::Phi));
    }
}

// The large payload makes accidental eager expansion visible in both load
// counts and graph size. A null guard checks that the tag read stays fixed.
fn tag_only_load(niche: bool) -> Body {
    let (mut types, word, boolean, _, _) = types();
    let payload = types.intern(
        TypeShape::Array {
            element: word,
            length: 4096,
        },
        Some(SizeAlign {
            size: 32768,
            align: 8,
        }),
    );
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "LargeTagOnly".into(),
        },
        Some(SizeAlign {
            size: 32776,
            align: 8,
        }),
    );
    types.record_aggregate(
        enumeration,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![],
                vec![],
                vec![FieldLayout {
                    ty: payload,
                    offset: 8,
                }],
            ],
        },
    );
    types.record_semantic_enum(enumeration, vec![3, 9, 27]);
    types.enums.insert(
        enumeration,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: word,
                offset: 0,
            }),
            encoding: if niche {
                TagEncoding::Niche {
                    untagged_variant: 2,
                    niche_variants: 0..=1,
                    niche_start: 100,
                }
            } else {
                TagEncoding::Direct {
                    discriminants: vec![3, 9, 27],
                }
            },
        },
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: enumeration,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut b = builder(
        if niche {
            "semantic_niche_tag"
        } else {
            "semantic_direct_tag"
        },
        types,
        vec![PassMode::Direct],
        PassMode::Direct,
    );
    let ret = local(&mut b, word);
    let pointer_arg = local(&mut b, pointer);
    b.set_arg_count(1);
    let is_null = local(&mut b, boolean);
    let null = constant(&mut b, pointer, 0);
    assign(
        &mut b,
        Place::local(is_null),
        Rvalue::BinaryOp(BinaryOp::Eq, copy(pointer_arg), null),
    );
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(is_null),
            targets: SwitchTargets {
                cases: vec![(1, BlockId(1))],
                otherwise: BlockId(2),
            },
        },
        false,
    );
    let missing = constant(&mut b, word, 999);
    assign(&mut b, Place::local(ret), Rvalue::Use(missing));
    b.finish_block(Terminator::Return, false);
    let source = Place {
        local: pointer_arg,
        projection: b.add_projection(vec![ProjectionElem::Deref]),
    };
    assign(&mut b, Place::local(ret), Rvalue::Discriminant(source));
    b.finish_block(Terminator::Return, false);
    b.finish()
}

#[test]
fn enum_discriminant_loads_only_the_fixed_tag_without_expanding_large_payloads() {
    for niche in [false, true] {
        let graph = lower(&tag_only_load(niche));
        assert_eq!(
            graph.ids().filter(|&n| graph.op(n) == Opcode::Load).count(),
            1
        );
        assert_eq!(
            graph.ids().filter(|&n| graph.op(n) == Opcode::If).count(),
            1
        );
        assert!(graph.stack_slots.is_empty());
        assert!(
            graph.len() < 150,
            "tag demand expanded the 4096-element payload"
        );
    }
}

#[test]
fn typed_observations_preserve_active_poison_without_memory_roundtrips() {
    use kfrc_son::{InterpretError, Interpreter, NodeId, SemanticOutcome, Value};

    // Products must observe every field, while an empty enum variant must not
    // observe the Undefined leaves that lowering creates for inactive payloads.
    for (is_enum, variant, poison) in [
        (false, 0, false),
        (false, 0, true),
        (true, 0, false),
        (true, 1, false),
        (true, 1, true),
    ] {
        let source = enum_functions().remove(0);
        let enumeration = source.locals[0].ty;
        let word = source.types.aggregates[&enumeration].variants[1][0].ty;
        let mut types = source.types;
        let pair = types.intern(
            TypeShape::Tuple(vec![word, word]),
            Some(SizeAlign { size: 16, align: 8 }),
        );
        let mut b = builder("observe_leaves", types, vec![], PassMode::Direct);
        let ret = local(&mut b, word);
        let zero = constant(&mut b, word, 0);
        assign(&mut b, Place::local(ret), Rvalue::Use(zero));
        b.finish_block(Terminator::Return, false);
        let mut graph = build_graph(&b.finish()).unwrap();
        let ret = graph
            .ids()
            .find(|&n| graph.op(n) == Opcode::Return)
            .unwrap();
        let good = graph.add_constant(42, word);
        let field = if poison {
            graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(word),
            )
        } else {
            good
        };
        let ty = if is_enum { enumeration } else { pair };
        let inputs = if is_enum {
            if variant == 0 { vec![] } else { vec![field] }
        } else {
            vec![good, field]
        };
        let value = graph.add_node(
            if is_enum {
                Opcode::MakeVariant
            } else {
                Opcode::Aggregate
            },
            NodeId::NONE,
            &inputs,
            variant,
            ValueType::Rust(ty),
        );
        let observed = graph.add_node(
            Opcode::RequireDefined,
            graph.control(ret),
            &[graph.input(ret, 0), value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        let control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[observed],
            0,
            ValueType::Control,
        );
        let effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[observed],
            1,
            ValueType::Effect,
        );
        let result = value;
        let projected = graph.add_node(
            if is_enum {
                Opcode::Discriminant
            } else {
                Opcode::Field
            },
            NodeId::NONE,
            &[result],
            0,
            ValueType::Rust(word),
        );
        graph.set_control(ret, control);
        graph.set_inputs(ret, &[effect, projected]);
        verify_graph(&mut graph).unwrap();
        let semantic = Interpreter::new(&graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| Ok(Value::Scalar(0)));
        lower_layout(&mut graph).unwrap();
        verify_graph(&mut graph).unwrap();
        assert!(graph.stack_slots.is_empty());
        assert!(
            !graph
                .ids()
                .any(|n| matches!(graph.op(n), Opcode::Load | Opcode::Store))
        );
        let layout = Interpreter::new(&graph).run(&[], &mut |_: &str, _: &[u64]| Ok(0));
        if poison {
            assert!(matches!(semantic, Err(InterpretError::PoisonObserved(_))));
            refinement::assert_release_refines(&semantic, &layout, |_, _| unreachable!());
        } else {
            let expected = if is_enum {
                if variant == 0 { 3 } else { 9 }
            } else {
                42
            };
            assert_eq!(
                semantic.unwrap(),
                SemanticOutcome::Return(Value::Scalar(expected))
            );
            assert_eq!(layout.unwrap(), kfrc_son::Outcome::Return(expected));
        }
    }
}

#[test]
fn layout_scalarizes_loop_values_without_materializing_local_storage() {
    let graph = lower(&fibonacci());
    assert!(graph.stack_slots.is_empty());
    assert!(graph.ids().any(|node| graph.op(node) == Opcode::Phi));
    assert!(
        !graph
            .ids()
            .any(|node| matches!(graph.op(node), Opcode::Load | Opcode::Store))
    );
}

fn repeat_store(length: u64, patch: bool) -> Body {
    let (mut types, word, _, _, _) = types();
    let array = types.intern(
        TypeShape::Array {
            element: word,
            length,
        },
        Some(SizeAlign {
            size: length * 8,
            align: 8,
        }),
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: array,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut b = builder(
        &format!("semantic_repeat_{length}_{}", u8::from(patch)),
        types,
        vec![PassMode::Direct, PassMode::Direct],
        PassMode::Direct,
    );
    let ret = local(&mut b, word);
    let destination = local(&mut b, pointer);
    let input = local(&mut b, word);
    b.set_arg_count(2);
    let value = local(&mut b, array);
    assign(
        &mut b,
        Place::local(value),
        Rvalue::Repeat(copy(input), length),
    );
    if patch {
        let place = field(&mut b, value, (length / 2) as u32, word);
        let replacement = constant(&mut b, word, 765);
        assign(&mut b, place, Rvalue::Use(replacement));
    }
    let destination = Place {
        local: destination,
        projection: b.add_projection(vec![ProjectionElem::Deref]),
    };
    assign(&mut b, destination, Rvalue::Use(copy(value)));
    assign(&mut b, Place::local(ret), Rvalue::Use(copy(input)));
    b.finish_block(Terminator::Return, false);
    b.finish()
}

#[test]
fn repeat_layout_work_is_bounded_by_updates_instead_of_array_length() {
    let mut sizes = Vec::new();
    for length in [0, 1, 16, 17, 256, 4096] {
        let body = repeat_store(length, false);
        let graph = build_graph(&body).unwrap();
        assert_eq!(
            graph
                .ids()
                .filter(|&n| graph.op(n) == Opcode::Repeat)
                .count(),
            1
        );
        assert!(graph.len() < 30);
        let graph = lower(&body);
        let stores = graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::Store)
            .count();
        assert_eq!(stores, if length <= 16 { length as usize } else { 1 });
        assert!(graph.stack_slots.is_empty());
        if length > 16 {
            sizes.push(graph.len());
        }
    }
    assert!(sizes.windows(2).all(|pair| pair[0] == pair[1]));
    let patched = lower(&repeat_store(4096, true));
    assert_eq!(
        patched
            .ids()
            .filter(|&n| patched.op(n) == Opcode::Store)
            .count(),
        3
    );
    assert!(patched.len() < 100);
}

fn repeat_choice(length: u64) -> Body {
    let (mut types, word, boolean, _, _) = types();
    let array = types.intern(
        TypeShape::Array {
            element: word,
            length,
        },
        Some(SizeAlign {
            size: length * 8,
            align: 8,
        }),
    );
    let mut b = builder(
        &format!("semantic_repeat_choice_{length}"),
        types,
        vec![PassMode::Direct; 3],
        PassMode::Indirect,
    );
    let ret = local(&mut b, array);
    let condition = local(&mut b, boolean);
    let yes = local(&mut b, word);
    let no = local(&mut b, word);
    b.set_arg_count(3);
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(condition),
            targets: SwitchTargets {
                cases: vec![(1, BlockId(1))],
                otherwise: BlockId(2),
            },
        },
        false,
    );
    assign(&mut b, Place::local(ret), Rvalue::Repeat(copy(yes), length));
    b.finish_block(Terminator::Goto(BlockId(3)), false);
    assign(&mut b, Place::local(ret), Rvalue::Repeat(copy(no), length));
    b.finish_block(Terminator::Goto(BlockId(3)), false);
    b.finish_block(Terminator::Return, false);
    b.finish()
}

#[test]
fn repeated_phi_and_indirect_return_keep_layout_and_machine_code_bounded() {
    let mut counts = Vec::new();
    for length in [256, 4096] {
        let graph = lower(&repeat_choice(length));
        assert_eq!(graph.stack_slots.len(), 1);
        assert_eq!(
            graph
                .ids()
                .filter(|&n| graph.op(n) == Opcode::Store)
                .count(),
            1
        );
        assert!(graph.len() < 100);
        let machine = lower_function(&graph).unwrap();
        assert!(machine.symbols.iter().any(|name| name == "memmove"));
        counts.push((graph.len(), machine.instructions.len()));
    }
    assert_eq!(counts[0], counts[1]);
}

#[test]
fn pointer_index_lowering_preserves_signed_wrapping_and_zero_sized_stride() {
    for size in [0, 8] {
        let (mut types, word, _, _, unit) = types();
        let signed = types.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: true,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: if size == 0 { unit } else { word },
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut b = builder(
            "signed_index",
            types,
            vec![PassMode::Direct; 2],
            PassMode::Direct,
        );
        let ret = local(&mut b, pointer);
        let base = local(&mut b, pointer);
        let index = local(&mut b, signed);
        b.set_arg_count(2);
        assign(
            &mut b,
            Place::local(ret),
            Rvalue::BinaryOp(BinaryOp::Offset, copy(base), copy(index)),
        );
        b.finish_block(Terminator::Return, false);
        let graph = lower(&b.finish());
        for (base, index) in [(0x1000u64, -2i64), (u64::MAX - 3, 1), (0, -1)] {
            let result = kfrc_son::Interpreter::new(&graph)
                .run(&[base, index as u64], &mut |_: &str, _: &[u64]| Ok(0))
                .unwrap();
            assert_eq!(
                result,
                kfrc_son::Outcome::Return(base.wrapping_add((index as u64).wrapping_mul(size)))
            );
        }
    }
}

#[test]
fn repeat_observation_ignores_empty_and_fully_overwritten_poison_bases() {
    use kfrc_son::{InterpretError, Interpreter, NodeId, SemanticOutcome, Value};
    for (length, replacements) in [(0, 0), (17, 0), (17, 1), (17, 17)] {
        let (mut types, word, _, _, _) = types();
        let array = types.intern(
            TypeShape::Array {
                element: word,
                length,
            },
            Some(SizeAlign {
                size: length * 8,
                align: 8,
            }),
        );
        let mut b = builder("observe_repeat", types, vec![], PassMode::Direct);
        let ret = local(&mut b, word);
        let zero = constant(&mut b, word, 0);
        assign(&mut b, Place::local(ret), Rvalue::Use(zero));
        b.finish_block(Terminator::Return, false);
        let mut graph = build_graph(&b.finish()).unwrap();
        let ret = graph
            .ids()
            .find(|&n| graph.op(n) == Opcode::Return)
            .unwrap();
        let poison = graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(word),
        );
        let mut value = graph.add_node(
            Opcode::Repeat,
            NodeId::NONE,
            &[poison],
            0,
            ValueType::Rust(array),
        );
        let initialized = graph.add_constant(42, word);
        for index in 0..replacements {
            value = graph.add_node(
                Opcode::InsertField,
                NodeId::NONE,
                &[value, initialized],
                index,
                ValueType::Rust(array),
            );
        }
        let observer = graph.add_node(
            Opcode::RequireDefined,
            graph.control(ret),
            &[graph.input(ret, 0), value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        let control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[observer],
            0,
            ValueType::Control,
        );
        let effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[observer],
            1,
            ValueType::Effect,
        );
        graph.set_control(ret, control);
        graph.set_input(ret, 0, effect);
        verify_graph(&mut graph).unwrap();
        let semantic = Interpreter::new(&graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| Ok(Value::Scalar(0)));
        lower_layout(&mut graph).unwrap();
        verify_graph(&mut graph).unwrap();
        assert!(graph.stack_slots.is_empty());
        let physical = Interpreter::new(&graph).run(&[], &mut |_: &str, _: &[u64]| Ok(0));
        if length == u64::from(replacements) {
            assert_eq!(semantic.unwrap(), SemanticOutcome::Return(Value::Scalar(0)));
            assert_eq!(physical.unwrap(), kfrc_son::Outcome::Return(0));
        } else {
            assert!(matches!(semantic, Err(InterpretError::PoisonObserved(_))));
            refinement::assert_release_refines(&semantic, &physical, |_, _| unreachable!());
        }
    }
}

#[test]
fn layout_materializes_only_explicit_abi_snapshots() {
    let graph = lower(&call_pair());
    assert_eq!(
        graph.stack_slots.len(),
        1,
        "only the outgoing argument is a graph value slot"
    );
    assert!(
        graph
            .ids()
            .any(|node| matches!(graph.value_type(node), ValueType::LayoutValue(_)))
    );
    lower(&swap_pair());
    let graph = lower(&snapshot_across_aliasing_call());
    assert_eq!(
        graph.stack_slots.len(),
        1,
        "returning an input uses its captured value, not the original address"
    );
}

#[test]
fn failed_layout_lowering_preserves_the_verified_input() {
    let (mut types, word, _, _, _) = types();
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "Choice".into(),
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_semantic_aggregate(
        enumeration,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![word], vec![]],
        },
    );
    types.record_semantic_enum(enumeration, vec![0, 1]);
    let mut b = builder(
        "unsupported_enum",
        types,
        vec![PassMode::Indirect],
        PassMode::Indirect,
    );
    let ret = local(&mut b, enumeration);
    let arg = local(&mut b, enumeration);
    b.set_arg_count(1);
    assign(&mut b, Place::local(ret), Rvalue::Use(copy(arg)));
    b.finish_block(Terminator::Return, false);
    let mut graph = build_graph(&b.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    let before = graph.dump();
    assert!(
        matches!(lower_layout(&mut graph), Err(LayoutError::Unsupported(message)) if message.contains("layout answer"))
    );
    assert_eq!(graph.dump(), before);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn encoded_layout_values_match_c_across_calls_and_loop_backedges() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::process::Command;
    let functions = [
        swap_pair(),
        call_pair(),
        fibonacci(),
        snapshot_across_aliasing_call(),
    ]
    .into_iter()
    .chain(enum_functions())
    .chain([tag_only_load(false), tag_only_load(true)])
    .chain([0, 1, 16, 17, 256, 4096].map(|n| repeat_store(n, false)))
    .chain([repeat_store(4096, true)])
    .chain([repeat_choice(256), repeat_choice(4096)])
    .map(|body| {
        let graph = lower(&body);
        FunctionCode {
            function: lower_function(&graph).unwrap(),
            binding: SymbolBinding::Global { hidden: false },
        }
    })
    .collect::<Vec<_>>();
    let dir = std::env::temp_dir().join(format!("kfrc-semantic-layout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let object = dir.join("semantic.o");
    let source = dir.join("main.c");
    let exe = dir.join("check");
    std::fs::write(&object, write_object(&functions).unwrap()).unwrap();
    std::fs::write(
        &source,
        r#"
#include <stdint.h>
struct pair { uint64_t a, b; };
struct triple { uint64_t a, b, c; };
extern struct pair semantic_swap(struct pair);
extern uint64_t semantic_call(struct pair);
extern uint64_t semantic_fibonacci(uint64_t);
extern void semantic_snapshot(struct triple *, const struct triple *, uint64_t *);
extern struct pair semantic_enum_identity(struct pair);
extern struct pair semantic_enum_select(_Bool, struct pair, struct pair);
extern uintptr_t semantic_niche_identity(uintptr_t);
extern uint64_t semantic_direct_tag(const void *);
extern uint64_t semantic_niche_tag(const void *);
extern uint64_t semantic_repeat_0_0(uint64_t *, uint64_t);
extern uint64_t semantic_repeat_1_0(uint64_t *, uint64_t);
extern uint64_t semantic_repeat_16_0(uint64_t *, uint64_t);
extern uint64_t semantic_repeat_17_0(uint64_t *, uint64_t);
extern uint64_t semantic_repeat_256_0(uint64_t *, uint64_t);
extern uint64_t semantic_repeat_4096_0(uint64_t *, uint64_t);
extern uint64_t semantic_repeat_4096_1(uint64_t *, uint64_t);
extern uint64_t *semantic_repeat_choice_256(uint64_t *, _Bool, uint64_t, uint64_t);
extern uint64_t *semantic_repeat_choice_4096(uint64_t *, _Bool, uint64_t, uint64_t);
void mutate_first(uint64_t *p) { *p = 999; }
int main(void) {
    uint64_t repeated[4098];
    uint64_t (*fill[])(uint64_t *,uint64_t) = {
        semantic_repeat_0_0, semantic_repeat_1_0, semantic_repeat_16_0,
        semantic_repeat_17_0, semantic_repeat_256_0, semantic_repeat_4096_0,
        semantic_repeat_4096_1
    };
    const unsigned lengths[] = {0,1,16,17,256,4096,4096};
    for (unsigned f=0; f<7; f++) for (uint64_t seed=0; seed<10; seed++) {
        for (unsigned i=0; i<4098; i++) repeated[i] = 0xdeadbeef;
        uint64_t value = seed * 0x1020304050607ULL;
        if (fill[f](repeated+1,value) != value) return 10;
        if (repeated[0] != 0xdeadbeef || repeated[lengths[f]+1] != 0xdeadbeef) return 11;
        for (unsigned i=0; i<lengths[f]; i++) {
            uint64_t expected = f == 6 && i == 2048 ? 765 : value;
            if (repeated[i+1] != expected) return 12;
        }
    }
    for (unsigned condition=0; condition<2; condition++) {
        repeated[0] = repeated[4097] = 0xdeadbeef;
        uint64_t *p = semantic_repeat_choice_4096(repeated+1, condition, 123, 456);
        if (p != repeated+1 || repeated[0] != 0xdeadbeef || repeated[4097] != 0xdeadbeef) return 13;
        for (unsigned i=1; i<=4096; i++) if (repeated[i] != (condition ? 123 : 456)) return 14;
        repeated[257] = 0xdeadbeef;
        p = semantic_repeat_choice_256(repeated+1, condition, 789, 321);
        if (p != repeated+1 || repeated[257] != 0xdeadbeef) return 15;
        for (unsigned i=1; i<=256; i++) if (repeated[i] != (condition ? 789 : 321)) return 16;
    }
    if (semantic_direct_tag(0) != 999 || semantic_niche_tag(0) != 999) return 7;
    uint64_t large[4097] = {0};
    const uint64_t direct_tags[] = {3,9,27}, niche_tags[] = {100,101,500};
    for (unsigned i=0; i<3; i++) {
        large[0] = direct_tags[i];
        if (semantic_direct_tag(large) != direct_tags[i]) return 8;
        large[0] = niche_tags[i];
        if (semantic_niche_tag(large) != direct_tags[i]) return 9;
    }
    for (uint64_t n=0; n<100; n++) {
        struct pair empty = {3, 0xdeadbeef}, data = {9, n*13};
        struct pair a = semantic_enum_identity(empty), b = semantic_enum_identity(data);
        if ((a.a & 255) != 3 || (b.a & 255) != 9 || b.b != n*13) return 4;
        a = semantic_enum_select(0, data, empty); b = semantic_enum_select(1, data, empty);
        if ((a.a & 255) != 3 || (b.a & 255) != 9 || b.b != n*13) return 5;
        if (semantic_niche_identity(n ? (uintptr_t)&data : 0) != (n ? (uintptr_t)&data : 0)) return 6;
    }
    struct triple input = {11,22,33}, output;
    semantic_snapshot(&output, &input, &input.a);
    if (input.a != 999 || output.a != 11 || output.b != 22 || output.c != 33) return 3;
    for (uint64_t i=0; i<100; i++) {
        struct pair p = {i*17, 99-i};
        struct pair q = semantic_swap(p);
        if (q.a != p.b || q.b != p.a || semantic_call(p) != p.b-p.a) return 1;
    }
    uint64_t a=0, b=1;
    for (uint64_t n=0; n<90; n++) {
        if (semantic_fibonacci(n) != a) return 2;
        uint64_t sum=a+b; a=b; b=sum;
    }
    return 0;
}
"#,
    )
    .unwrap();
    let linked = Command::new("cc")
        .args(["-O2", "-o"])
        .arg(&exe)
        .arg(&source)
        .arg(&object)
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let ran = Command::new(&exe).output().unwrap();
    assert!(ran.status.success(), "machine difference: {:?}", ran.status);
    std::fs::remove_dir_all(&dir).unwrap();
}
