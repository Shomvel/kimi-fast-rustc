//! Enum layout work follows actual payloads instead of every declared variant.
use kfrc_body::*;
use kfrc_son::{FunctionGraph, Opcode, build_graph, lower_layout, verify_graph};
use kfrc_x86::lower_function;

fn enum_body(nested: bool, known: bool) -> Body {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let kind = types.intern(
        TypeShape::Adt {
            name: "EightWayKind".into(),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    types.record_aggregate(
        kind,
        AggregateLayout {
            fields: vec![],
            variants: vec![vec![]; 8],
        },
    );
    types.record_semantic_enum(kind, (0..8).collect());
    types.enums.insert(
        kind,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: word,
                offset: 0,
            }),
            encoding: TagEncoding::Direct {
                discriminants: (0..8).collect(),
            },
        },
    );
    let outer = types.intern(
        TypeShape::Adt {
            name: "Outer".into(),
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        outer,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![],
                vec![FieldLayout {
                    ty: kind,
                    offset: 8,
                }],
            ],
        },
    );
    types.record_semantic_enum(outer, vec![0, 1]);
    types.enums.insert(
        outer,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: word,
                offset: 0,
            }),
            encoding: TagEncoding::Direct {
                discriminants: vec![0, 1],
            },
        },
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
    let ty = if nested { outer } else { kind };
    let pass = if nested {
        PassMode::Pair
    } else {
        PassMode::Direct
    };
    let mut body = BodyBuilder::new(
        format!("enum_{}_{}", u8::from(nested), u8::from(known)),
        FunctionAbi {
            args: if known { vec![] } else { vec![pass.clone()] },
            ret: pass,
            c_variadic: false,
        },
        types,
    );
    let ret = body.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    });
    if known {
        let value = body.add_local(LocalDecl {
            ty: kind,
            mutability: Mutability::Mut,
            name: None,
        });
        body.push_statement(StatementKind::Assign(
            Place::local(value),
            Rvalue::Aggregate(
                AggregateKind::Adt {
                    ty: kind,
                    variant: VariantIndex(6),
                },
                vec![],
            ),
        ));
        let value = Operand::Copy(Place::local(value));
        body.push_statement(StatementKind::Assign(
            Place::local(ret),
            if nested {
                Rvalue::Aggregate(
                    AggregateKind::Adt {
                        ty: outer,
                        variant: VariantIndex(1),
                    },
                    vec![value],
                )
            } else {
                Rvalue::Use(value)
            },
        ));
    } else {
        let input = body.add_local(LocalDecl {
            ty,
            mutability: Mutability::Mut,
            name: None,
        });
        body.set_arg_count(1);
        body.push_statement(StatementKind::Assign(
            Place::local(ret),
            Rvalue::Use(Operand::Copy(Place::local(input))),
        ));
    }
    body.finish_block(Terminator::Return, false);
    body.finish()
}

fn lower(body: &Body) -> FunctionGraph {
    let mut graph = build_graph(body).unwrap();
    verify_graph(&mut graph).unwrap();
    lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

#[test]
fn known_nested_and_fieldless_dynamic_enums_do_not_build_variant_trees() {
    for (nested, known, branches, stores) in [
        (false, false, 0, 1),
        (false, true, 0, 1),
        (true, true, 0, 2),
        (true, false, 2, 2),
    ] {
        let graph = lower(&enum_body(nested, known));
        assert_eq!(
            graph.ids().filter(|&n| graph.op(n) == Opcode::If).count(),
            branches
        );
        assert_eq!(
            graph
                .ids()
                .filter(|&n| graph.op(n) == Opcode::Store)
                .count(),
            stores
        );
        assert!(
            !graph
                .ids()
                .any(|n| matches!(graph.op(n), Opcode::BitAnd | Opcode::BitOr | Opcode::Negate))
        );
        lower_function(&graph).unwrap();
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn encoded_nested_enum_materialization_matches_c() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::process::Command;
    let functions =
        [(false, false), (false, true), (true, true), (true, false)].map(|(nested, known)| {
            FunctionCode {
                function: lower_function(&lower(&enum_body(nested, known))).unwrap(),
                binding: SymbolBinding::Global { hidden: false },
            }
        });
    let directory =
        std::env::temp_dir().join(format!("kfrc-enum-materialization-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let object = directory.join("enum.o");
    let source = directory.join("main.c");
    let executable = directory.join("check");
    std::fs::write(&object, write_object(&functions).unwrap()).unwrap();
    std::fs::write(
        &source,
        r#"
#include <stdint.h>
struct outer { uint64_t tag, payload; };
extern uint64_t enum_0_0(uint64_t);
extern uint64_t enum_0_1(void);
extern struct outer enum_1_1(void);
extern struct outer enum_1_0(struct outer);
int main(void) {
    if (enum_0_1() != 6) return 1;
    struct outer known = enum_1_1();
    if (known.tag != 1 || known.payload != 6) return 2;
    for (uint64_t tag=0; tag<8; tag++) {
        if (enum_0_0(tag) != tag) return 3;
        struct outer input = {1, tag}, output = enum_1_0(input);
        if (output.tag != 1 || output.payload != tag) return 4;
        input.tag = 0; output = enum_1_0(input);
        if (output.tag != 0) return 5;
    }
    return 0;
}
"#,
    )
    .unwrap();
    let linked = Command::new("cc")
        .arg("-O2")
        .arg(&source)
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let result = Command::new(&executable).output().unwrap();
    assert!(
        result.status.success(),
        "machine mismatch {:?}",
        result.status
    );
    std::fs::remove_dir_all(directory).unwrap();
}
