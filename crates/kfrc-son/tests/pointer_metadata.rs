//! Thin metadata is typed unit and does not load the pointer representation.

use kfrc_body::*;
use kfrc_son::{BuildError, Interpreter, Opcode, Outcome, build_graph, verify_graph};

fn local(builder: &mut BodyBuilder, ty: TyId) -> LocalId {
    builder.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}

fn copy(local: LocalId) -> Operand {
    Operand::Copy(Place::local(local))
}

fn callee(builder: &mut BodyBuilder, unit: TyId, symbol: &str, args: Vec<PassMode>) -> Operand {
    Operand::Constant(builder.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::Function(Callee::Function {
            symbol: symbol.into(),
            abi: FunctionAbi {
                args,
                ret: PassMode::Direct,
                c_variadic: false,
            },
        }),
    }))
}

#[test]
fn thin_pointer_and_reference_metadata_is_unit_without_loads_and_keeps_call_effects() {
    for pointer_kind in [
        "const",
        "mut",
        "ref",
        "ref_mut",
        "function_pointer",
        "extern",
    ] {
        let mut types = RecordedTypes::new();
        let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pointee = match pointer_kind {
            // Function pointers are represented as thin raw pointers by the
            // rustc converter; a pointer to one is thin as well.
            "function_pointer" => types.intern(
                TypeShape::RawPointer {
                    mutability: Mutability::Not,
                    pointee: unit,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            ),
            "extern" => types.intern(
                TypeShape::Other {
                    name: "ForeignType".into(),
                },
                None,
            ),
            _ => word,
        };
        if pointer_kind == "extern" {
            types.record_thin_unsized(pointee);
        }
        let shape = match pointer_kind {
            "ref" | "ref_mut" => TypeShape::Reference {
                mutability: if pointer_kind == "ref_mut" {
                    Mutability::Mut
                } else {
                    Mutability::Not
                },
                pointee,
            },
            _ => TypeShape::RawPointer {
                mutability: if pointer_kind == "mut" {
                    Mutability::Mut
                } else {
                    Mutability::Not
                },
                pointee,
            },
        };
        let pointer = types.intern(shape, Some(SizeAlign { size: 8, align: 8 }));
        let indirect = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: pointer,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let abi = FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        };
        let mut builder = BodyBuilder::new(pointer_kind, abi, types);
        let ret = local(&mut builder, word);
        let produced = local(&mut builder, pointer);
        let address = local(&mut builder, indirect);
        let metadata = local(&mut builder, unit);
        let produce = callee(&mut builder, unit, "produce_pointer", vec![]);
        builder.finish_block(
            Terminator::Call {
                callee: produce,
                args: vec![],
                destination: Place::local(produced),
                target: Some(BlockId(1)),
                unwind: UnwindAction::Unreachable,
            },
            false,
        );
        builder.push_statement(StatementKind::Assign(
            Place::local(address),
            Rvalue::RawPointer(Mutability::Not, Place::local(produced)),
        ));
        let deref = builder.add_projection(vec![ProjectionElem::Deref]);
        builder.push_statement(StatementKind::Assign(
            Place::local(metadata),
            Rvalue::UnaryOp(
                UnaryOp::PtrMetadata,
                Operand::Copy(Place {
                    local: address,
                    projection: deref,
                }),
            ),
        ));
        let consume = callee(
            &mut builder,
            unit,
            "consume_metadata",
            vec![PassMode::Ignore],
        );
        builder.finish_block(
            Terminator::Call {
                callee: consume,
                args: vec![copy(metadata)],
                destination: Place::local(ret),
                target: Some(BlockId(2)),
                unwind: UnwindAction::Unreachable,
            },
            false,
        );
        builder.finish_block(Terminator::Return, false);
        let mut graph = build_graph(&builder.finish()).unwrap();
        verify_graph(&mut graph).unwrap();
        assert!(
            !graph.ids().any(|node| graph.op(node) == Opcode::Load),
            "{pointer_kind}"
        );
        let mut seen = Vec::new();
        let mut calls = |symbol: &str, args: &[u64]| {
            assert!(args.is_empty());
            seen.push(symbol.to_owned());
            Ok(42)
        };
        assert_eq!(
            Interpreter::new(&layout_graph(&graph))
                .run(&[], &mut calls)
                .unwrap(),
            Outcome::Return(42)
        );
        assert_eq!(seen, ["produce_pointer", "consume_metadata"]);
    }
}

#[test]
fn wide_pointer_metadata_does_not_fall_back_to_thin_unit_without_its_lane() {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slice = types.intern(TypeShape::Slice { element: word }, None);
    let wide = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: slice,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        wide,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: word,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    let abi = FunctionAbi {
        args: vec![],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut builder = BodyBuilder::new("malformed_wide_metadata", abi, types);
    let ret = local(&mut builder, word);
    let pointer = local(&mut builder, wide);
    builder.push_statement(StatementKind::Assign(
        Place::local(ret),
        Rvalue::UnaryOp(UnaryOp::PtrMetadata, copy(pointer)),
    ));
    builder.finish_block(Terminator::Return, false);
    let result = build_graph(&builder.finish());
    assert!(
        matches!(&result, Err(BuildError::Unsupported { what, .. }) if what.contains("invalid logical field None/1")),
        "malformed wide pointer must reject its missing metadata field: {result:?}"
    );
}

/// Exercise the unique value pipeline before interpreting its physical ABI.
fn layout_graph(graph: &kfrc_son::FunctionGraph) -> kfrc_son::FunctionGraph {
    let mut graph = graph.clone();
    kfrc_son::verify_graph(&mut graph).unwrap();
    kfrc_son::lower_layout(&mut graph).unwrap();
    kfrc_son::verify_graph(&mut graph).unwrap();
    graph
}
