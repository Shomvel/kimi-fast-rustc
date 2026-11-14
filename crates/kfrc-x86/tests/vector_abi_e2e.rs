//! Vector source fixtures are rejected; raw XMM encoders remain executable.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::process::Command;

use kfrc_body::*;
use kfrc_son::{BuildError, build_graph};
use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
use kfrc_x86::machine::x86::{MachineFunction, MachineInstr, Reg, Xmm};

fn local(body: &mut BodyBuilder, ty: TyId) -> LocalId {
    body.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}
fn copy(local: LocalId) -> Operand {
    Operand::Copy(Place::local(local))
}
fn types() -> (RecordedTypes, TyId, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let array = types.intern(
        TypeShape::Array {
            element: word,
            length: 2,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    let vector = types.intern(
        TypeShape::Adt {
            name: "__m128i".into(),
        },
        Some(SizeAlign {
            size: 16,
            align: 16,
        }),
    );
    types.record_aggregate(
        vector,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: array,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    types.record_vector(
        vector,
        VectorLayout {
            element: word,
            lanes: 2,
        },
    );
    let triple = types.intern(
        TypeShape::Adt {
            name: "Triple".into(),
        },
        Some(SizeAlign { size: 24, align: 8 }),
    );
    types.record_aggregate(
        triple,
        AggregateLayout {
            fields: (0..3)
                .map(|i| FieldLayout {
                    ty: word,
                    offset: i * 8,
                })
                .collect(),
            variants: vec![],
        },
    );
    (types, word, vector, triple)
}
fn mixed_arguments(word: TyId, vector: TyId) -> Vec<(TyId, PassMode)> {
    let mut args = Vec::new();
    for _ in 0..7 {
        args.extend([
            (vector, PassMode::Vector { bytes: 16 }),
            (word, PassMode::Direct),
        ]);
    }
    // The ninth vector follows one spilled integer and requires a stack gap;
    // the tenth follows another integer and requires a second stack gap.
    args.extend([
        (vector, PassMode::Vector { bytes: 16 }),
        (vector, PassMode::Vector { bytes: 16 }),
        (word, PassMode::Direct),
        (vector, PassMode::Vector { bytes: 16 }),
    ]);
    args
}
fn forward(symbol: &str, scalar: bool, indirect: bool, sret: bool) -> Body {
    let (mut types, word, vector, triple) = types();
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let args = mixed_arguments(word, vector);
    let ret_ty = if sret {
        triple
    } else if scalar {
        word
    } else {
        vector
    };
    let ret = if sret {
        PassMode::Indirect
    } else if scalar {
        PassMode::Direct
    } else {
        PassMode::Vector { bytes: 16 }
    };
    let callee_abi = FunctionAbi {
        args: args.iter().map(|(_, mode)| *mode).collect(),
        ret,
        c_variadic: false,
    };
    let mut abi = callee_abi.clone();
    if indirect {
        abi.args.insert(0, PassMode::Direct);
    }
    let mut body = BodyBuilder::new(symbol, abi, types);
    let result = local(&mut body, ret_ty);
    let callback = if indirect {
        Some(local(&mut body, pointer))
    } else {
        None
    };
    let params = args
        .iter()
        .map(|(ty, _)| local(&mut body, *ty))
        .collect::<Vec<_>>();
    body.set_arg_count(params.len() + usize::from(indirect));
    let callee = body.add_constant(Constant {
        ty: pointer,
        kind: ConstantKind::Function(Callee::Function {
            symbol: if sret {
                "c_vector_sret"
            } else if scalar {
                "c_vector_scalar"
            } else {
                "c_vector_mix"
            }
            .into(),
            abi: callee_abi.clone(),
        }),
    });
    let arguments = params.iter().copied().map(copy).collect();
    let call = if let Some(callback) = callback {
        Terminator::CallIndirect {
            function: copy(callback),
            abi: callee_abi.clone(),
            args: arguments,
            destination: Place::local(result),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        }
    } else {
        Terminator::Call {
            callee: Operand::Constant(callee),
            args: arguments,
            destination: Place::local(result),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        }
    };
    body.finish_block(call, false);
    if !scalar && !sret {
        // Consume original inputs again after a C call has clobbered all XMMs.
        let mut arguments: Vec<_> = params.iter().copied().map(copy).collect();
        arguments[0] = copy(result);
        body.finish_block(
            Terminator::Call {
                callee: Operand::Constant(callee),
                args: arguments,
                destination: Place::local(result),
                target: Some(BlockId(2)),
                unwind: UnwindAction::Unreachable,
            },
            false,
        );
    }
    body.finish_block(Terminator::Return, false);
    body.finish()
}
fn pick_stack_vector() -> Body {
    let (types, word, vector, _) = types();
    let args = mixed_arguments(word, vector);
    let mut body = BodyBuilder::new(
        "vector_pick_stack",
        FunctionAbi {
            args: args.iter().map(|(_, mode)| *mode).collect(),
            ret: PassMode::Vector { bytes: 16 },
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut body, vector);
    let params = args
        .iter()
        .map(|(ty, _)| local(&mut body, *ty))
        .collect::<Vec<_>>();
    body.set_arg_count(params.len());
    body.push_statement(StatementKind::Assign(
        Place::local(ret),
        Rvalue::Use(copy(*params.last().unwrap())),
    ));
    body.finish_block(Terminator::Return, false);
    body.finish()
}
fn extended_register_spill() -> FunctionCode {
    let mut function = MachineFunction::new("vector_extended_spill");
    function.stack_slots = 2;
    function.callee_saved = vec![Reg::R12];
    function.instructions = vec![
        MachineInstr::StackAddress {
            dst: Reg::R12,
            offset: -16,
        },
        MachineInstr::StoreVector {
            address: Reg::R12,
            src: Xmm(0),
        },
        MachineInstr::LoadVector {
            dst: Xmm(15),
            address: Reg::R12,
        },
        MachineInstr::StoreVector {
            address: Reg::R12,
            src: Xmm(15),
        },
        MachineInstr::LoadVector {
            dst: Xmm(0),
            address: Reg::R12,
        },
        MachineInstr::Ret,
    ];
    FunctionCode {
        function,
        binding: SymbolBinding::Global { hidden: false },
    }
}

#[test]
fn vector_abi_input_fixtures_are_explicitly_rejected() {
    for body in [
        forward("vector_forward", false, false, false),
        forward("vector_indirect", false, true, false),
        forward("vector_scalar", true, false, false),
        forward("vector_sret", false, false, true),
        pick_stack_vector(),
    ] {
        assert!(
            matches!(build_graph(&body), Err(BuildError::Unsupported { what, .. })
            if what.to_lowercase().contains("vector") || what.contains("SIMD"))
        );
    }
}

#[test]
fn raw_vector_extended_register_spill_matches_c_in_both_emitters() {
    let functions = [extended_register_spill()];
    let object = write_object(&functions).unwrap();
    let assembly = functions
        .iter()
        .map(|f| f.function.emit_gnu_assembly().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let dir = std::env::temp_dir().join(format!(
        "kfrc-vector-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("vector.o"), object).unwrap();
    std::fs::write(dir.join("vector.s"), assembly).unwrap();
    std::fs::write(
        dir.join("driver.c"),
        r#"
#include <stdint.h>
#include <stdio.h>
#include <emmintrin.h>
extern __m128i vector_extended_spill(__m128i);
int main(void) {
    for (uint64_t seed=0;seed<256;seed++) {
        __m128i value=_mm_set_epi64x(~seed,seed);
        __m128i got=vector_extended_spill(value);
        if (_mm_movemask_epi8(_mm_cmpeq_epi8(value,got)) != 65535) return 1;
    }
    puts("raw XMM extended register spill PASS");
    return 0;
}
"#,
    )
    .unwrap();
    for (input, executable) in [("vector.o", "encoded"), ("vector.s", "assembled")] {
        let link = Command::new("cc")
            .current_dir(&dir)
            .args(["driver.c", input, "-O1", "-msse2", "-o", executable])
            .output()
            .unwrap();
        std::fs::write(dir.join(format!("{executable}.link.stdout")), &link.stdout).unwrap();
        std::fs::write(dir.join(format!("{executable}.link.stderr")), &link.stderr).unwrap();
        assert!(
            link.status.success(),
            "{executable}: {}",
            String::from_utf8_lossy(&link.stderr)
        );
        let run = Command::new(dir.join(executable)).output().unwrap();
        std::fs::write(dir.join(format!("{executable}.run.stdout")), &run.stdout).unwrap();
        std::fs::write(dir.join(format!("{executable}.run.stderr")), &run.stderr).unwrap();
        assert!(
            run.status.success(),
            "{executable}: {:?}: {}",
            run.status,
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(String::from_utf8_lossy(&run.stdout).contains("PASS"));
    }
    if let Some(destination) = std::env::var_os("KFRC_TEST_ARTIFACT_DIR") {
        let destination = std::path::PathBuf::from(destination).join(dir.file_name().unwrap());
        std::fs::create_dir_all(&destination).unwrap();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
