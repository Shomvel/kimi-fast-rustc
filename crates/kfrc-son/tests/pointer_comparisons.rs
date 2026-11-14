//! Pointer representation casts preserve the exact type used by comparisons.

use kfrc_body::*;
use kfrc_son::{Interpreter, Outcome, build_graph, optimize_graph, verify_graph};

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

fn pointer(types: &mut RecordedTypes, pointee: TyId, mutability: Mutability, size: u64) -> TyId {
    types.intern(
        TypeShape::RawPointer {
            mutability,
            pointee,
        },
        Some(SizeAlign { size, align: 8 }),
    )
}

#[test]
fn thin_raw_pointer_casts_keep_the_comparison_type_before_and_after_optimization() {
    for op in [
        BinaryOp::Eq,
        BinaryOp::Ne,
        BinaryOp::Lt,
        BinaryOp::Le,
        BinaryOp::Gt,
        BinaryOp::Ge,
    ] {
        for target_mutability in [Mutability::Not, Mutability::Mut] {
            let mut types = RecordedTypes::new();
            let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
            let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
            let left_ty = pointer(&mut types, unit, target_mutability, 8);
            let other_ty = pointer(
                &mut types,
                unit,
                if target_mutability == Mutability::Not {
                    Mutability::Mut
                } else {
                    Mutability::Not
                },
                8,
            );
            let abi = FunctionAbi {
                args: vec![PassMode::Direct; 2],
                ret: PassMode::Direct,
                c_variadic: false,
            };
            let mut builder = BodyBuilder::new("thin_pointer_compare", abi, types);
            let ret = local(&mut builder, boolean);
            let left = local(&mut builder, left_ty);
            let right = local(&mut builder, other_ty);
            builder.set_arg_count(2);
            let normalized = local(&mut builder, left_ty);
            builder.push_statement(StatementKind::Assign(
                Place::local(normalized),
                Rvalue::Cast(CastKind::PtrToPtr, copy(right), left_ty),
            ));
            builder.push_statement(StatementKind::Assign(
                Place::local(ret),
                Rvalue::BinaryOp(op, copy(left), copy(normalized)),
            ));
            builder.finish_block(Terminator::Return, false);
            let mut graph = build_graph(&builder.finish()).unwrap();
            for optimized in [false, true] {
                if optimized {
                    optimize_graph(&mut graph);
                }
                verify_graph(&mut graph).unwrap();
                for (left, right) in [
                    (0, 0),
                    (0x1000, 0x1010),
                    (u64::MAX, 0),
                    (0, u64::MAX),
                    (0x8000_0000_0000_0000, 0x7fff_ffff_ffff_ffff),
                ] {
                    let expected = match op {
                        BinaryOp::Eq => left == right,
                        BinaryOp::Ne => left != right,
                        BinaryOp::Lt => left < right,
                        BinaryOp::Le => left <= right,
                        BinaryOp::Gt => left > right,
                        BinaryOp::Ge => left >= right,
                        _ => unreachable!(),
                    };
                    let mut calls = |_: &str, _: &[u64]| unreachable!();
                    assert_eq!(
                        Interpreter::new(&layout_graph(&graph))
                            .run(&[left, right], &mut calls)
                            .unwrap(),
                        Outcome::Return(u64::from(expected)),
                        "{op:?} {target_mutability:?} optimized={optimized}"
                    );
                }
            }
        }
    }
}

#[test]
fn fat_raw_pointer_casts_preserve_data_metadata_and_exact_comparison_types() {
    for op in [
        BinaryOp::Eq,
        BinaryOp::Ne,
        BinaryOp::Lt,
        BinaryOp::Le,
        BinaryOp::Gt,
        BinaryOp::Ge,
    ] {
        let mut types = RecordedTypes::new();
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let slice = types.intern(TypeShape::Slice { element: byte }, None);
        let data = pointer(&mut types, byte, Mutability::Not, 8);
        let left_ty = pointer(&mut types, slice, Mutability::Not, 16);
        let right_ty = pointer(&mut types, slice, Mutability::Mut, 16);
        let fields = [
            FieldLayout {
                ty: data,
                offset: 0,
            },
            FieldLayout {
                ty: word,
                offset: 8,
            },
        ];
        for ty in [left_ty, right_ty] {
            types.record_aggregate(
                ty,
                AggregateLayout {
                    fields: fields.to_vec(),
                    variants: vec![],
                },
            );
            types.record_abi_pair(ty, fields);
        }
        let abi = FunctionAbi {
            args: vec![PassMode::Pair; 2],
            ret: PassMode::Direct,
            c_variadic: false,
        };
        let mut builder = BodyBuilder::new("fat_pointer_compare", abi, types);
        let ret = local(&mut builder, boolean);
        let left = local(&mut builder, left_ty);
        let right = local(&mut builder, right_ty);
        builder.set_arg_count(2);
        let normalized = local(&mut builder, left_ty);
        builder.push_statement(StatementKind::Assign(
            Place::local(normalized),
            Rvalue::Cast(CastKind::Transmute, copy(right), left_ty),
        ));
        builder.push_statement(StatementKind::Assign(
            Place::local(ret),
            Rvalue::BinaryOp(op, copy(left), copy(normalized)),
        ));
        builder.finish_block(Terminator::Return, false);
        let mut graph = build_graph(&builder.finish()).unwrap();
        for optimized in [false, true] {
            if optimized {
                optimize_graph(&mut graph);
            }
            verify_graph(&mut graph).unwrap();
            for (left, right) in [
                ((0x1000, 0), (0x1000, 0)),
                ((0x1000, 1), (0x1000, 2)),
                ((0x1000, 9), (0x2000, 0)),
                ((u64::MAX, 0), (0, u64::MAX)),
            ] {
                let expected = match op {
                    BinaryOp::Eq => left == right,
                    BinaryOp::Ne => left != right,
                    BinaryOp::Lt => left < right,
                    BinaryOp::Le => left <= right,
                    BinaryOp::Gt => left > right,
                    BinaryOp::Ge => left >= right,
                    _ => unreachable!(),
                };
                let mut calls = |_: &str, _: &[u64]| unreachable!();
                assert_eq!(
                    Interpreter::new(&layout_graph(&graph))
                        .run(&[left.0, left.1, right.0, right.1], &mut calls)
                        .unwrap(),
                    Outcome::Return(u64::from(expected)),
                    "{op:?} optimized={optimized}"
                );
            }
        }
    }
}

/// Exercise the unique value pipeline before interpreting its physical ABI.
fn layout_graph(graph: &kfrc_son::FunctionGraph) -> kfrc_son::FunctionGraph {
    let mut graph = graph.clone();
    kfrc_son::verify_graph(&mut graph).unwrap();
    kfrc_son::lower_layout(&mut graph).unwrap();
    kfrc_son::verify_graph(&mut graph).unwrap();
    graph
}
