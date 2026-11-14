use super::*;
use crate::machine::eval;
use kfrc_body::{FunctionAbi, RecordedTypes};

fn reg(n: u32) -> VirtualReg {
    VirtualReg(n)
}
fn v(n: u32) -> VirtualValue {
    VirtualValue::Reg(reg(n))
}
fn imm(n: i64) -> VirtualValue {
    VirtualValue::Imm(n)
}
fn scalar(i: VirtualInstr) -> LowerInstr {
    LowerInstr::Scalar(i)
}
fn root() -> LowerInstr {
    LowerInstr::StackAddress {
        dst: reg(0),
        slot: 0,
    }
}
fn store(bytes: u8, src: VirtualValue) -> LowerInstr {
    LowerInstr::Store {
        address: v(0),
        offset: 0,
        src,
        bytes,
    }
}
fn load(bytes: u8) -> LowerInstr {
    LowerInstr::Load {
        dst: reg(2),
        address: v(0),
        offset: 0,
        bytes,
    }
}
fn layouts() -> Vec<SizeAlign> {
    vec![SizeAlign { size: 16, align: 8 }]
}

fn compile(code: Vec<LowerInstr>, slots: &[SizeAlign], name: &str) -> MachineFunction {
    let graph = FunctionGraph::new(
        name,
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    allocated::compile(&graph, code, 16, slots).unwrap()
}

#[test]
fn narrow_round_trips_truncate_dirty_high_bits_and_release_the_frame() {
    for bytes in [1, 2, 4, 8] {
        let mut code = vec![
            root(),
            scalar(VirtualInstr::LoadArg {
                dst: reg(1),
                index: 0,
            }),
            store(bytes, v(1)),
            load(bytes),
            scalar(VirtualInstr::Return(v(2))),
        ];
        let mut slots = layouts();
        let stats = run(&mut code, &mut slots);
        assert_eq!(
            stats,
            Stats {
                loads_forwarded: 1,
                stores_removed: 1,
                slots_removed: 1
            }
        );
        assert!(slots.is_empty());
        let function = compile(code, &slots, "narrow");
        assert_eq!(function.stack_slots, 0);
        function.emit_machine_code().unwrap();
        for input in [0, -1, i64::MIN, 0x123456789abcdef] {
            let expected = ((input as u64) & (u64::MAX >> (64 - u32::from(bytes) * 8))) as i64;
            assert_eq!(eval::evaluate(&function, &[input]).unwrap(), expected);
        }
    }
}

#[test]
fn exact_overwrites_and_adjacent_lanes_keep_the_latest_snapshot() {
    let mut code = vec![
        root(),
        scalar(VirtualInstr::Add {
            dst: reg(3),
            lhs: v(0),
            rhs: imm(8),
        }),
        store(8, imm(11)),
        store(8, imm(17)),
        LowerInstr::Store {
            address: v(3),
            offset: 0,
            src: imm(29),
            bytes: 8,
        },
        load(8),
        LowerInstr::Load {
            dst: reg(4),
            address: v(3),
            offset: 0,
            bytes: 8,
        },
        scalar(VirtualInstr::Add {
            dst: reg(5),
            lhs: v(2),
            rhs: v(4),
        }),
        scalar(VirtualInstr::Return(v(5))),
    ];
    let mut slots = layouts();
    assert_eq!(run(&mut code, &mut slots).loads_forwarded, 2);
    let function = compile(code, &slots, "lanes");
    assert_eq!(eval::evaluate(&function, &[]).unwrap(), 46);
    assert_eq!(function.stack_slots, 0);
}

fn unchanged(mut code: Vec<LowerInstr>) {
    let before = format!("{code:?}");
    let mut slots = layouts();
    assert_eq!(run(&mut code, &mut slots), Stats::default());
    assert_eq!(format!("{code:?}"), before);
    assert_eq!(slots, layouts());
}

#[test]
fn partial_writes_uninitialized_reads_and_redefined_sources_keep_storage() {
    unchanged(vec![root(), store(4, imm(7)), load(8)]);
    unchanged(vec![root(), load(8)]);
    unchanged(vec![
        root(),
        store(8, imm(7)),
        scalar(VirtualInstr::Add {
            dst: reg(3),
            lhs: v(0),
            rhs: imm(1),
        }),
        LowerInstr::Store {
            address: v(3),
            offset: 0,
            src: imm(9),
            bytes: 1,
        },
        load(8),
    ]);
    unchanged(vec![
        root(),
        scalar(VirtualInstr::Mov {
            dst: reg(1),
            src: imm(7),
        }),
        store(8, v(1)),
        scalar(VirtualInstr::Mov {
            dst: reg(1),
            src: imm(9),
        }),
        load(8),
    ]);
}

#[test]
fn calls_labels_and_back_edges_do_not_forward_across_control_segments() {
    for boundary in [
        scalar(VirtualInstr::Label(1)),
        scalar(VirtualInstr::Jmp(1)),
        scalar(VirtualInstr::Call {
            dst: reg(5),
            target: "opaque".into(),
            args: vec![],
        }),
        LowerInstr::AbiCall {
            results: vec![],
            target: CallDestination::Direct("opaque".into()),
            args: vec![],
            vector_args: vec![],
            vector_result: None,
        },
    ] {
        unchanged(vec![root(), store(8, imm(7)), boundary, load(8)]);
    }
}

#[test]
fn escaping_slot_addresses_keep_owned_storage() {
    for escape in [
        scalar(VirtualInstr::Return(v(0))),
        LowerInstr::CallIndirect {
            dst: reg(5),
            target: v(1),
            args: vec![v(0)],
        },
        LowerInstr::AbiCall {
            results: vec![],
            target: CallDestination::Direct("opaque".into()),
            args: vec![(0, v(0))],
            vector_args: vec![],
            vector_result: None,
        },
        LowerInstr::Store {
            address: v(1),
            offset: 0,
            src: v(0),
            bytes: 8,
        },
        LowerInstr::Store {
            address: v(0),
            offset: 0,
            src: v(0),
            bytes: 8,
        },
        LowerInstr::ParallelCopy(vec![(reg(3), v(0))]),
        LowerInstr::LoadVector {
            dst: Xmm(0),
            address: v(0),
        },
        scalar(VirtualInstr::Add {
            dst: reg(3),
            lhs: v(0),
            rhs: v(1),
        }),
    ] {
        unchanged(vec![root(), store(8, imm(7)), load(8), escape]);
    }
}

#[test]
fn dead_stores_leave_traps_and_calls_intact_and_remap_surviving_slots() {
    let mut code = vec![
        root(),
        store(8, imm(7)),
        scalar(VirtualInstr::UDiv {
            dst: reg(4),
            lhs: imm(1),
            rhs: imm(0),
        }),
        LowerInstr::StackAddress {
            dst: reg(5),
            slot: 1,
        },
        scalar(VirtualInstr::Return(v(5))),
    ];
    let mut slots = vec![
        SizeAlign { size: 16, align: 8 },
        SizeAlign { size: 8, align: 8 },
    ];
    assert_eq!(run(&mut code, &mut slots).slots_removed, 1);
    assert_eq!(slots, vec![SizeAlign { size: 8, align: 8 }]);
    assert!(
        code.iter()
            .any(|i| matches!(i, LowerInstr::Scalar(VirtualInstr::UDiv { .. })))
    );
    assert!(code.iter().any(|i| matches!(
        i,
        LowerInstr::StackAddress {
            dst: VirtualReg(5),
            slot: 0
        }
    )));
}

#[test]
fn unused_address_chains_release_slots_without_a_load_or_store() {
    for with_address in [false, true] {
        let mut code = if with_address {
            vec![
                root(),
                scalar(VirtualInstr::Mov {
                    dst: reg(3),
                    src: v(0),
                }),
                scalar(VirtualInstr::Add {
                    dst: reg(4),
                    lhs: v(3),
                    rhs: imm(4),
                }),
            ]
        } else {
            Vec::new()
        };
        code.push(scalar(VirtualInstr::Return(imm(1))));
        let mut slots = layouts();
        assert_eq!(
            run(&mut code, &mut slots),
            Stats {
                loads_forwarded: 0,
                stores_removed: 0,
                slots_removed: 1,
            }
        );
        let function = compile(code, &slots, "unused_slot");
        assert_eq!(function.stack_slots, 0);
        assert_eq!(eval::evaluate(&function, &[]).unwrap(), 1);
    }
}

#[test]
fn rs1_values_forward_without_dropping_their_machine_contracts() {
    let mut code = vec![
        root(),
        scalar(VirtualInstr::LoadArg {
            dst: reg(1),
            index: 0,
        }),
        LowerInstr::Rotate {
            dst: reg(3),
            value: v(1),
            amount: imm(7),
            width: BitWidth::W64,
            right: false,
        },
        store(8, v(3)),
        load(8),
        scalar(VirtualInstr::Return(v(2))),
    ];
    let mut slots = layouts();
    assert_eq!(run(&mut code, &mut slots).loads_forwarded, 1);
    let function = compile(code, &slots, "rotate");
    assert_eq!(
        eval::evaluate(&function, &[0x12345]).unwrap(),
        0x12345i64.rotate_left(7)
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn encoded_round_trips_match_the_original_memory_operations() {
    use crate::{FunctionCode, SymbolBinding, write_object};
    use std::process::Command;
    let mut functions = Vec::new();
    for bytes in [1, 2, 4, 8] {
        let original = vec![
            root(),
            scalar(VirtualInstr::LoadArg {
                dst: reg(1),
                index: 0,
            }),
            store(bytes, v(1)),
            load(bytes),
            scalar(VirtualInstr::Return(v(2))),
        ];
        let mut candidate = original.clone();
        let mut slots = layouts();
        run(&mut candidate, &mut slots);
        for (name, code, frame) in [
            (format!("old{bytes}"), original, layouts()),
            (format!("new{bytes}"), candidate, slots),
        ] {
            functions.push(FunctionCode {
                function: compile(code, &frame, &name),
                binding: SymbolBinding::Global { hidden: false },
            });
        }
    }
    let dir = std::env::temp_dir().join(format!("kfrc-private-slots-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("slots.o"), write_object(&functions).unwrap()).unwrap();
    let mut source = String::from("#include <stdint.h>\n");
    for bytes in [1, 2, 4, 8] {
        source.push_str(&format!(
            "extern uint64_t old{bytes}(uint64_t), new{bytes}(uint64_t);\n"
        ));
    }
    source.push_str(
        "int main(void) { uint64_t x=1; for(int i=0;i<10000;i++) { x=x*6364136223846793005ULL+1;\n",
    );
    for bytes in [1, 2, 4, 8] {
        source.push_str(&format!(
            "if(old{bytes}(x)!=new{bytes}(x)) return {bytes};\n"
        ));
    }
    source.push_str("} return 0; }\n");
    std::fs::write(dir.join("driver.c"), source).unwrap();
    let linked = Command::new("cc")
        .current_dir(&dir)
        .args(["driver.c", "slots.o", "-o", "driver"])
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let result = Command::new(dir.join("driver")).output().unwrap();
    assert!(result.status.success(), "{:?}", result.status);
    std::fs::remove_dir_all(dir).unwrap();
}
