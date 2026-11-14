//! Linux executes the real encoded objects, including calls made by C.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::process::Command;

use kfrc_body::*;
use kfrc_son::{build_graph, lower_layout, verify_graph};
use kfrc_x86::{FunctionCode, SymbolBinding, lower_function, write_object};

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

fn assign(builder: &mut BodyBuilder, place: Place, value: Rvalue) {
    builder.push_statement(StatementKind::Assign(place, value));
}

fn constant(builder: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(builder.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(bits),
    }))
}

fn compile(body: Body) -> FunctionCode {
    let mut graph = build_graph(&body).unwrap_or_else(|error| panic!("{error}\n{body}"));
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    lower_layout(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    FunctionCode {
        function: lower_function(&graph)
            .unwrap_or_else(|error| panic!("{error}\n{}", graph.dump())),
        binding: SymbolBinding::Global { hidden: false },
    }
}

fn scalar_types() -> (RecordedTypes, TyId, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let i64 = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: true,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let u64 = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    (types, i64, u64, boolean)
}

#[test]
fn encoded_memory_aliasing_scalar_pairs_and_checked_products_match_c() {
    let (mut types, i64, u64, boolean) = scalar_types();
    let pointer = types.intern(
        TypeShape::Reference {
            mutability: Mutability::Mut,
            pointee: i64,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let pair = types.intern(
        TypeShape::Tuple(vec![i64, i64]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        pair,
        AggregateLayout {
            fields: vec![
                FieldLayout { ty: i64, offset: 0 },
                FieldLayout { ty: i64, offset: 8 },
            ],
            variants: vec![],
        },
    );
    types.record_abi_pair(
        pair,
        [
            FieldLayout { ty: i64, offset: 0 },
            FieldLayout { ty: i64, offset: 8 },
        ],
    );
    let overflow_pair = types.intern(
        TypeShape::Tuple(vec![u64, boolean]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        overflow_pair,
        AggregateLayout {
            fields: vec![
                FieldLayout { ty: u64, offset: 0 },
                FieldLayout {
                    ty: boolean,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );

    let mut alias = BodyBuilder::new(
        "alias",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types.clone(),
    );
    let ret = local(&mut alias, i64);
    let arg = local(&mut alias, i64);
    alias.set_arg_count(1);
    let value = local(&mut alias, i64);
    let reference = local(&mut alias, pointer);
    let next = local(&mut alias, i64);
    assign(&mut alias, Place::local(value), Rvalue::Use(copy(arg)));
    assign(
        &mut alias,
        Place::local(reference),
        Rvalue::Ref(BorrowKind::Mut, Place::local(value)),
    );
    let one = constant(&mut alias, i64, 1);
    assign(
        &mut alias,
        Place::local(next),
        Rvalue::BinaryOp(BinaryOp::Add, copy(arg), one),
    );
    let deref = Place {
        local: reference,
        projection: alias.add_projection(vec![ProjectionElem::Deref]),
    };
    assign(&mut alias, deref, Rvalue::Use(copy(next)));
    assign(&mut alias, Place::local(ret), Rvalue::Use(copy(value)));
    alias.finish_block(Terminator::Return, false);

    let mut swap = BodyBuilder::new(
        "swap_pair",
        FunctionAbi {
            args: vec![PassMode::Pair],
            ret: PassMode::Pair,
            c_variadic: false,
        },
        types.clone(),
    );
    let ret = local(&mut swap, pair);
    let arg = local(&mut swap, pair);
    swap.set_arg_count(1);
    let first = Place {
        local: arg,
        projection: swap.add_projection(vec![ProjectionElem::Field(FieldIndex(0), i64)]),
    };
    let second = Place {
        local: arg,
        projection: swap.add_projection(vec![ProjectionElem::Field(FieldIndex(1), i64)]),
    };
    assign(
        &mut swap,
        Place::local(ret),
        Rvalue::Aggregate(
            AggregateKind::Tuple,
            vec![Operand::Copy(second), Operand::Copy(first)],
        ),
    );
    swap.finish_block(Terminator::Return, false);

    let triple = types.intern(
        TypeShape::Tuple(vec![i64, i64, i64]),
        Some(SizeAlign { size: 24, align: 8 }),
    );
    types.record_aggregate(
        triple,
        AggregateLayout {
            fields: vec![
                FieldLayout { ty: i64, offset: 0 },
                FieldLayout { ty: i64, offset: 8 },
                FieldLayout {
                    ty: i64,
                    offset: 16,
                },
            ],
            variants: vec![],
        },
    );
    let mut indirect = BodyBuilder::new(
        "rotate_triple",
        FunctionAbi {
            args: vec![PassMode::Indirect],
            ret: PassMode::Indirect,
            c_variadic: false,
        },
        types.clone(),
    );
    let ret = local(&mut indirect, triple);
    let arg = local(&mut indirect, triple);
    indirect.set_arg_count(1);
    let mut operands = Vec::new();
    for field in [1, 2, 0] {
        let projection =
            indirect.add_projection(vec![ProjectionElem::Field(FieldIndex(field), i64)]);
        operands.push(Operand::Copy(Place {
            local: arg,
            projection,
        }));
    }
    assign(
        &mut indirect,
        Place::local(ret),
        Rvalue::Aggregate(AggregateKind::Tuple, operands),
    );
    indirect.finish_block(Terminator::Return, false);

    let mut overflow = BodyBuilder::new(
        "multiply_overflows",
        FunctionAbi {
            args: vec![PassMode::Direct, PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut overflow, boolean);
    let lhs = local(&mut overflow, u64);
    let rhs = local(&mut overflow, u64);
    overflow.set_arg_count(2);
    let result = local(&mut overflow, overflow_pair);
    assign(
        &mut overflow,
        Place::local(result),
        Rvalue::BinaryOp(BinaryOp::MulWithOverflow, copy(lhs), copy(rhs)),
    );
    let flag = Place {
        local: result,
        projection: overflow.add_projection(vec![ProjectionElem::Field(FieldIndex(1), boolean)]),
    };
    assign(
        &mut overflow,
        Place::local(ret),
        Rvalue::Use(Operand::Copy(flag)),
    );
    overflow.finish_block(Terminator::Return, false);

    let object = write_object(&[
        compile(alias.finish()),
        compile(swap.finish()),
        compile(overflow.finish()),
        compile(indirect.finish()),
    ])
    .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "kfrc-memory-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("memory.o"), object).unwrap();
    std::fs::write(
        dir.join("driver.c"),
        r#"
#include <stdint.h>
#include <stdio.h>
struct pair { int64_t first, second; };
extern int64_t alias(int64_t);
extern struct pair swap_pair(struct pair);
extern unsigned char multiply_overflows(uint64_t, uint64_t);
struct triple { int64_t a,b,c; };
extern void rotate_triple(struct triple*, const struct triple*);
int main(void) {
  for (int64_t i = -100; i <= 100; i++) {
    if (alias(i) != i+1) return 1;
    struct pair result = swap_pair((struct pair){i, -i-7});
    if (result.first != -i-7 || result.second != i) return 2;
  }
  struct triple source = {-19,20,71}, destination;
  rotate_triple(&destination,&source);
  if (destination.a != 20 || destination.b != 71 || destination.c != -19) return 4;
  uint64_t inputs[] = {0,1,2,UINT32_MAX,UINT64_MAX,UINT64_C(1)<<63};
  for (unsigned i=0;i<6;i++) for(unsigned j=0;j<6;j++) {
    uint64_t product;
    int flag = __builtin_mul_overflow(inputs[i], inputs[j], &product);
    if (multiply_overflows(inputs[i],inputs[j]) != flag) return 3;
  }
  puts("memory/pair/checked-mul PASS");
  return 0;
}
"#,
    )
    .unwrap();
    let compiler = Command::new("cc")
        .current_dir(&dir)
        .args(["driver.c", "memory.o", "-o", "driver"])
        .output()
        .unwrap();
    assert!(
        compiler.status.success(),
        "link failed: {}",
        String::from_utf8_lossy(&compiler.stderr)
    );
    let run = Command::new(dir.join("driver")).output().unwrap();
    assert!(
        run.status.success(),
        "driver status {:?}; stderr {}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("PASS"));
    std::fs::remove_dir_all(dir).unwrap();
}
