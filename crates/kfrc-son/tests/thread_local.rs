//! Existing TLS counter inputs remain explicit out-of-scope rejection probes.

use kfrc_body::*;
use kfrc_son::{build_graph, BuildError};

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

fn counter(template: bool, conditional: bool, address_return: bool) -> Body {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let abi = FunctionAbi {
        args: if conditional {
            vec![PassMode::Direct]
        } else {
            vec![]
        },
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut builder = BodyBuilder::new("thread_counter", abi, types);
    let ret = local(&mut builder, if address_return { pointer } else { word });
    if conditional {
        let condition = local(&mut builder, boolean);
        builder.set_arg_count(1);
        builder.finish_block(
            Terminator::SwitchInt {
                discriminant: copy(condition),
                targets: SwitchTargets {
                    cases: vec![(1, BlockId(1))],
                    otherwise: BlockId(2),
                },
            },
            false,
        );
    }
    let address = local(&mut builder, pointer);
    let repeated = local(&mut builder, pointer);
    let value = local(&mut builder, word);
    if template {
        builder.add_data_object(DataObject {
            symbol: "tls_counter".into(),
            binding: DataBinding::Global { hidden: true },
            bytes: 41_u64.to_le_bytes().to_vec(),
            align: 8,
            writable: true,
            thread_local: true,
            unnamed_address: false,
            relocations: vec![],
        });
    }
    builder.push_statement(StatementKind::Assign(
        Place::local(address),
        Rvalue::ThreadLocalRef {
            symbol: "tls_counter".into(),
        },
    ));
    let deref = builder.add_projection(vec![ProjectionElem::Deref]);
    builder.push_statement(StatementKind::Assign(
        Place::local(value),
        Rvalue::Use(Operand::Copy(Place {
            local: address,
            projection: deref,
        })),
    ));
    let one = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(1),
    }));
    builder.push_statement(StatementKind::Assign(
        Place::local(value),
        Rvalue::BinaryOp(BinaryOp::Add, copy(value), one),
    ));
    builder.push_statement(StatementKind::Assign(
        Place {
            local: address,
            projection: deref,
        },
        Rvalue::Use(copy(value)),
    ));
    builder.push_statement(StatementKind::Assign(
        Place::local(repeated),
        Rvalue::ThreadLocalRef {
            symbol: "tls_counter".into(),
        },
    ));
    builder.push_statement(StatementKind::Assign(
        Place::local(ret),
        if address_return {
            Rvalue::Use(copy(repeated))
        } else {
            Rvalue::Use(Operand::Copy(Place {
                local: repeated,
                projection: deref,
            }))
        },
    ));
    builder.finish_block(Terminator::Return, false);
    if conditional {
        let zero = Operand::Constant(builder.add_constant(Constant {
            ty: word,
            kind: ConstantKind::Scalar(0),
        }));
        builder.push_statement(StatementKind::Assign(Place::local(ret), Rvalue::Use(zero)));
        builder.finish_block(Terminator::Return, false);
    }
    builder.finish()
}

#[test]
fn tls_counter_fixtures_are_rejected_before_graph_construction() {
    for template in [false, true] {
        for conditional in [false, true] {
            for address_return in [false, true] {
                let body = counter(template, conditional, address_return);
                let result = build_graph(&body);
                assert!(
                    matches!(&result, Err(BuildError::Unsupported { what, .. })
                    if what.contains("TLS") || what.contains("thread-local") || what.contains("ThreadLocal")),
                    "template={template}, conditional={conditional}, address_return={address_return}: {result:?}"
                );
            }
        }
    }
}
