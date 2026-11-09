//! Per-call integer Cast ABI lanes exercised against the platform C compiler.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use kfrc_body::*;
use kfrc_son::{build_graph, lower_layout, verify_graph};
use kfrc_x86::{FunctionCode, SymbolBinding, lower_function, write_object};
use std::process::Command;

fn local(body: &mut BodyBuilder, ty: TyId) -> LocalId {
    body.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}
fn constant(body: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(body.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(bits),
    }))
}
fn function(body: Body) -> FunctionCode {
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
fn cast(
    size: u64,
    bytes: &[u8],
    consecutive: bool,
    pad_i32: bool,
    extension: CastExtension,
) -> PassMode {
    let mut offset = 0;
    let mut lanes = [None, None];
    for (index, bytes) in bytes.iter().enumerate() {
        lanes[index] = Some(CastLane {
            offset,
            bytes: *bytes,
        });
        offset += u64::from(*bytes);
    }
    PassMode::Cast(CastAbi {
        size,
        lanes,
        consecutive,
        pad_i32,
        extension,
    })
}
fn identity(types: RecordedTypes, ty: TyId, mode: PassMode, symbol: &str) -> Body {
    let mut body = BodyBuilder::new(
        symbol,
        FunctionAbi {
            args: vec![mode],
            ret: mode,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut body, ty);
    let arg = local(&mut body, ty);
    body.set_arg_count(1);
    body.push_statement(StatementKind::Assign(
        Place::local(ret),
        Rvalue::Use(Operand::Copy(Place::local(arg))),
    ));
    body.finish_block(Terminator::Return, false);
    body.finish()
}

#[test]
fn cast_lanes_partial_tails_padding_extensions_and_register_exhaustion_match_c() {
    let mut types = RecordedTypes::new();
    let u8 = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let i8 = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: true,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let u64 = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let bytes3 = types.intern(
        TypeShape::Array {
            element: u8,
            length: 3,
        },
        Some(SizeAlign { size: 3, align: 1 }),
    );
    let bytes11 = types.intern(
        TypeShape::Array {
            element: u8,
            length: 11,
        },
        Some(SizeAlign { size: 11, align: 1 }),
    );
    let cast3 = cast(3, &[3], false, false, CastExtension::None);
    let cast11 = cast(11, &[8, 8], true, false, CastExtension::None);
    let mut functions = vec![
        function(identity(types.clone(), bytes3, cast3, "cast3_identity")),
        function(identity(types.clone(), bytes11, cast11, "cast11_identity")),
        function(identity(
            types.clone(),
            bytes3,
            cast(3, &[3], false, true, CastExtension::None),
            "cast3_padded",
        )),
    ];
    let mut modes = vec![PassMode::Direct; 5];
    modes.extend([cast11, PassMode::Direct]);
    let abi = FunctionAbi {
        args: modes,
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut body = BodyBuilder::new("cast_many", abi.clone(), types.clone());
    let ret = local(&mut body, u64);
    let mut arguments = Vec::new();
    for ty in [u64, u64, u64, u64, u64, bytes11, u64] {
        arguments.push(Operand::Copy(Place::local(local(&mut body, ty))));
    }
    body.set_arg_count(arguments.len());
    let function_ty = body.types().intern(
        TypeShape::FunctionItem {
            name: "c_many".to_owned(),
        },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    let callee = Operand::Constant(body.add_constant(Constant {
        ty: function_ty,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "c_many".to_owned(),
            abi,
        }),
    }));
    body.finish_block(
        Terminator::Call {
            callee,
            args: arguments,
            destination: Place::local(ret),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    body.finish_block(Terminator::Return, false);
    functions.push(function(body.finish()));

    for (name, callee, ty, extension) in [
        ("cast_sign_probe", "c_sign_abi", i8, CastExtension::Sign),
        ("cast_zero_probe", "c_zero_abi", u8, CastExtension::Zero),
    ] {
        let mut body = BodyBuilder::new(
            name,
            FunctionAbi {
                args: vec![],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types.clone(),
        );
        let ret = local(&mut body, u64);
        let value = constant(&mut body, ty, 255);
        let callee_ty = body.types().intern(
            TypeShape::FunctionItem {
                name: callee.to_owned(),
            },
            Some(SizeAlign { size: 0, align: 1 }),
        );
        let callee = Operand::Constant(body.add_constant(Constant {
            ty: callee_ty,
            kind: ConstantKind::Function(Callee::Function {
                symbol: callee.to_owned(),
                abi: FunctionAbi {
                    args: vec![cast(1, &[1], false, false, extension)],
                    ret: PassMode::Direct,
                    c_variadic: false,
                },
            }),
        }));
        body.finish_block(
            Terminator::Call {
                callee,
                args: vec![value],
                destination: Place::local(ret),
                target: Some(BlockId(1)),
                unwind: UnwindAction::Unreachable,
            },
            false,
        );
        body.finish_block(Terminator::Return, false);
        functions.push(function(body.finish()));
    }
    let object = write_object(&functions).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "kfrc-cast-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("cast.o"), object).unwrap();
    std::fs::write(directory.join("driver.c"),r#"
#include <stdint.h>
#include <string.h>
#include <stdio.h>
struct bytes3 { unsigned char bytes[3]; };
struct bytes11 { unsigned char bytes[11]; };
extern struct bytes3 cast3_identity(struct bytes3);
extern struct bytes11 cast11_identity(struct bytes11);
extern struct bytes3 cast3_padded(unsigned,struct bytes3);
extern uint64_t cast_many(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,struct bytes11,uint64_t);
extern uint64_t cast_sign_probe(void),cast_zero_probe(void);
uint64_t c_sign_abi(signed char x) { uint64_t raw; __asm__ volatile("movq %%rdi,%0":"=r"(raw)); return raw; }
uint64_t c_zero_abi(unsigned char x) { uint64_t raw; __asm__ volatile("movq %%rdi,%0":"=r"(raw)); return raw; }
uint64_t c_many(uint64_t a,uint64_t b,uint64_t c,uint64_t d,uint64_t e,struct bytes11 value,uint64_t tail) {
  uint64_t sum=a+2*b+3*c+4*d+5*e+7*tail;
  for(unsigned i=0;i<11;i++) sum+=(i+1)*value.bytes[i];
  return sum;
}
int main(void) {
  for(unsigned seed=0;seed<100;seed++) {
    struct bytes3 first={{seed,seed+7,255-seed}};
    struct bytes11 second;
    for(unsigned i=0;i<11;i++) second.bytes[i]=seed*3+i*17;
    struct bytes3 a=cast3_identity(first),b=cast3_padded(0xdeadbeef,first);
    struct bytes11 c=cast11_identity(second);
    if(memcmp(&a,&first,3)||memcmp(&b,&first,3)||memcmp(&c,&second,11)) return 1;
    uint64_t expected=c_many(11,22,33,44,55,second,seed+66);
    if(cast_many(11,22,33,44,55,second,seed+66)!=expected) return 2;
  }
  if(cast_sign_probe()!=UINT64_MAX||cast_zero_probe()!=255) return 3;
  puts("Cast partial/pad/extension/consecutive PASS");
  return 0;
}
"#).unwrap();
    let compiler = Command::new("cc")
        .current_dir(&directory)
        .args(["-O0", "driver.c", "cast.o", "-o", "driver"])
        .output()
        .unwrap();
    assert!(
        compiler.status.success(),
        "{}",
        String::from_utf8_lossy(&compiler.stderr)
    );
    let output = Command::new(directory.join("driver")).output().unwrap();
    assert!(
        output.status.success(),
        "driver {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("PASS"));
    std::fs::remove_dir_all(directory).unwrap();
}
