//! Over-aligned addressable locals keep the ordinary incoming/call stack ABI.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::process::Command;

use kfrc_body::*;
use kfrc_son::{build_graph, lower_layout, verify_graph};
use kfrc_x86::{FunctionCode, SymbolBinding, lower_function, write_object};
use kfrc_x86::machine::x86::{MachineInstr, Reg};

fn local(body: &mut BodyBuilder, ty: TyId) -> LocalId {
    body.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}

fn constant(body: &mut BodyBuilder, ty: TyId, value: u128) -> Operand {
    Operand::Constant(body.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(value),
    }))
}

fn scalar_call_function() -> FunctionCode {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let abi = FunctionAbi {
        args: vec![PassMode::Direct; 8],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut body = BodyBuilder::new("scalar_call_eight", abi.clone(), types);
    let result = local(&mut body, word);
    let arguments: Vec<_> = (0..8).map(|_| local(&mut body, word)).collect();
    body.set_arg_count(8);
    let callee = body.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "sum_eight_entry".into(),
            abi,
        }),
    });
    body.finish_block(
        Terminator::Call {
            callee: Operand::Constant(callee),
            args: arguments
                .iter()
                .map(|arg| Operand::Copy(Place::local(*arg)))
                .collect(),
            destination: Place::local(result),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    for argument in arguments {
        body.push_statement(StatementKind::Assign(
            Place::local(result),
            Rvalue::BinaryOp(
                BinaryOp::Add,
                Operand::Copy(Place::local(result)),
                Operand::Copy(Place::local(argument)),
            ),
        ));
    }
    body.finish_block(Terminator::Return, false);
    let mut graph = build_graph(&body.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    lower_layout(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    let function = lower_function(&graph).unwrap();
    assert!(
        !function.callee_saved.is_empty(),
        "scalar path must exercise saved registers with outgoing stack arguments"
    );
    FunctionCode {
        function,
        binding: SymbolBinding::Global { hidden: false },
    }
}

fn aligned_function(alignment: u64) -> FunctionCode {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let size = alignment * 2;
    let storage_type = types.intern(
        TypeShape::Adt {
            name: format!("Aligned{alignment}"),
        },
        Some(SizeAlign {
            size,
            align: alignment,
        }),
    );
    types.record_aggregate(
        storage_type,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: word,
                    offset: 0,
                },
                FieldLayout {
                    ty: word,
                    offset: size - 8,
                },
            ],
            variants: vec![],
        },
    );
    let mut body = BodyBuilder::new(
        format!("aligned_{alignment}"),
        FunctionAbi {
            args: vec![PassMode::Direct; 8],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let result = local(&mut body, word);
    let args: Vec<_> = (0..8).map(|_| local(&mut body, word)).collect();
    body.set_arg_count(8);
    let storage = local(&mut body, storage_type);
    let sentinel = local(&mut body, word);
    let storage_pointer = local(&mut body, pointer);
    let sentinel_pointer = local(&mut body, pointer);
    let first = Place {
        local: storage,
        projection: body.add_projection(vec![ProjectionElem::Field(FieldIndex(0), word)]),
    };
    let last = Place {
        local: storage,
        projection: body.add_projection(vec![ProjectionElem::Field(FieldIndex(1), word)]),
    };
    for (place, argument) in [
        (first, args[0]),
        (last, args[7]),
        (Place::local(sentinel), args[3]),
    ] {
        body.push_statement(StatementKind::Assign(
            place,
            Rvalue::Use(Operand::Copy(Place::local(argument))),
        ));
    }
    body.push_statement(StatementKind::Assign(
        Place::local(storage_pointer),
        Rvalue::RawPointer(Mutability::Mut, first),
    ));
    body.push_statement(StatementKind::Assign(
        Place::local(sentinel_pointer),
        Rvalue::RawPointer(Mutability::Mut, Place::local(sentinel)),
    ));
    let callee = body.add_constant(Constant {
        ty: pointer,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "check_aligned_entry".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct; 12],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        }),
    });
    let align = constant(&mut body, word, u128::from(alignment));
    let length = constant(&mut body, word, u128::from(size));
    let mut call_args = vec![
        Operand::Copy(Place::local(storage_pointer)),
        Operand::Copy(Place::local(sentinel_pointer)),
        align,
        length,
    ];
    call_args.extend(args.iter().map(|arg| Operand::Copy(Place::local(*arg))));
    body.finish_block(
        Terminator::Call {
            callee: Operand::Constant(callee),
            args: call_args,
            destination: Place::local(result),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    for extra in [first, last, Place::local(sentinel)] {
        body.push_statement(StatementKind::Assign(
            Place::local(result),
            Rvalue::BinaryOp(
                BinaryOp::Add,
                Operand::Copy(Place::local(result)),
                Operand::Copy(extra),
            ),
        ));
    }
    body.finish_block(Terminator::Return, false);
    let body = body.finish();
    let mut graph = build_graph(&body).unwrap();
    verify_graph(&mut graph).unwrap();
    lower_layout(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    let mut function = lower_function(&graph).unwrap();
    // Exercise additional clobbers without discarding the allocator's saves.
    // The memory path now uses R13–R15 as well; replacing this set produces an
    // invalid MachineFunction before either emitter sees the frame contract.
    for register in [Reg::Rbx, Reg::R12] {
        if !function.callee_saved.contains(&register) {
            function.callee_saved.push(register);
        }
    }
    function.instructions.insert(
        0,
        MachineInstr::MovImm {
            dst: Reg::R12,
            value: -91,
        },
    );
    function.instructions.insert(
        0,
        MachineInstr::MovImm {
            dst: Reg::Rbx,
            value: -17,
        },
    );
    FunctionCode {
        function,
        binding: SymbolBinding::Global { hidden: false },
    }
}

#[test]
fn over_aligned_slots_preserve_arguments_adjacent_storage_and_calls() {
    let mut functions: Vec<_> = [32, 64, 256, 4096]
        .into_iter()
        .map(aligned_function)
        .collect();
    functions.push(scalar_call_function());
    let object = write_object(&functions).unwrap();
    let assembly = functions
        .iter()
        .map(|code| code.function.emit_gnu_assembly().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let dir = std::env::temp_dir().join(format!(
        "kfrc-stack-align-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("aligned.o"), object).unwrap();
    std::fs::write(dir.join("aligned.s"), assembly).unwrap();
    std::fs::write(dir.join("driver.c"), r#"
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <alloca.h>
typedef uint64_t (*probe)(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t);
extern uint64_t aligned_32(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t);
extern uint64_t aligned_64(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t);
extern uint64_t aligned_256(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t);
extern uint64_t aligned_4096(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t);
extern uint64_t scalar_call_eight(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t);
__asm__(".text\n.globl check_aligned_entry\ncheck_aligned_entry:\n"
        "lea 8(%rsp), %rax\ntest $15, %rax\njnz .Lbad_call_alignment\n"
        "jmp check_aligned_c\n.Lbad_call_alignment:\nud2\n");
__asm__(".text\n.globl sum_eight_entry\nsum_eight_entry:\n"
        "lea 8(%rsp), %rax\ntest $15, %rax\njnz .Lbad_sum_alignment\n"
        "jmp sum_eight_c\n.Lbad_sum_alignment:\nud2\n");
uint64_t sum_eight_c(uint64_t a0,uint64_t a1,uint64_t a2,uint64_t a3,uint64_t a4,uint64_t a5,uint64_t a6,uint64_t a7) {
    return a0+a1+a2+a3+a4+a5+a6+a7;
}
uint64_t check_aligned_c(uint64_t *storage, uint64_t *sentinel, uint64_t alignment, uint64_t size,
                        uint64_t a0,uint64_t a1,uint64_t a2,uint64_t a3,uint64_t a4,uint64_t a5,uint64_t a6,uint64_t a7) {
    uint64_t *last = storage + size/8 - 1;
    if ((uintptr_t)storage % alignment || *storage != a0 || *last != a7 || *sentinel != a3) abort();
    if ((uintptr_t)sentinel >= (uintptr_t)storage && (uintptr_t)sentinel < (uintptr_t)storage + size) abort();
    *storage = a0+3; *last = a7+5; *sentinel = a3+1;
    return a0+a1+a2+a3+a4+a5+a6+a7;
}
__attribute__((noinline)) static int invoke(probe function, uint64_t seed, uint64_t factor, uint64_t addend) {
    volatile unsigned char *padding = alloca((seed % 256 + 1) * 16);
    padding[0] = (unsigned char)seed;
    register uint64_t saved_bx __asm__("rbx") = UINT64_C(0x13579bdf02468ace);
    register uint64_t saved __asm__("r12") = UINT64_C(0xabcdef1234567890);
    register uint64_t saved13 __asm__("r13") = UINT64_C(0x123456789abcdef0);
    register uint64_t saved14 __asm__("r14") = UINT64_C(0x8877665544332211);
    register uint64_t saved15 __asm__("r15") = UINT64_C(0xfedcba0987654321);
    __asm__ volatile("" : "+r"(saved_bx), "+r"(saved), "+r"(saved13), "+r"(saved14), "+r"(saved15));
    uint64_t result = function(seed,seed+1,seed+2,seed+3,seed+4,seed+5,seed+6,seed+7);
    __asm__ volatile("" : "+r"(saved_bx), "+r"(saved), "+r"(saved13), "+r"(saved14), "+r"(saved15));
    int preserved = saved_bx == UINT64_C(0x13579bdf02468ace) && saved == UINT64_C(0xabcdef1234567890)
        && saved13 == UINT64_C(0x123456789abcdef0) && saved14 == UINT64_C(0x8877665544332211)
        && saved15 == UINT64_C(0xfedcba0987654321);
    int ok = result == factor*seed+addend && preserved && padding[0] == (unsigned char)seed;
    if (!ok) fprintf(stderr, "seed=%llu result=%llu expected=%llu preserved=%d padding=%u\n",
        (unsigned long long)seed, (unsigned long long)result, (unsigned long long)(factor*seed+addend), preserved, padding[0]);
    return ok;
}
int main(void) {
    probe functions[] = {aligned_32,aligned_64,aligned_256,aligned_4096};
    for (unsigned i=0;i<4;i++) for (uint64_t seed=0;seed<256;seed++) if (!invoke(functions[i], seed, 11, 47)) return 1;
    for (uint64_t seed=0;seed<256;seed++) if (!invoke(scalar_call_eight, seed, 16, 56)) return 2;
    puts("aligned stack/arguments/calls/sentinels PASS");
    return 0;
}
"#).unwrap();
    for (input, executable) in [("aligned.o", "encoded"), ("aligned.s", "assembled")] {
        let link = Command::new("cc")
            .current_dir(&dir)
            .args(["driver.c", input, "-O1", "-o", executable])
            .output()
            .unwrap();
        assert!(
            link.status.success(),
            "{input}: {}",
            String::from_utf8_lossy(&link.stderr)
        );
        let run = Command::new(dir.join(executable)).output().unwrap();
        assert!(
            run.status.success(),
            "{executable}: {:?}: {}",
            run.status,
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(String::from_utf8_lossy(&run.stdout).contains("PASS"));
    }
    std::fs::remove_dir_all(dir).unwrap();
}
