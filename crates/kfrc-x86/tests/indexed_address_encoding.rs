//! Compare SIB bytes with an independent assembler, including R12 as index.
use kfrc_x86::machine::x86::{MachineFunction, MachineInstr, MemoryAddress, Reg, Scale};

fn encoded(instruction: MachineInstr) -> Vec<u8> {
    let mut function = MachineFunction::new("indexed");
    function.instructions = vec![instruction, MachineInstr::Ret];
    let mut bytes = function.emit_machine_code().unwrap();
    assert_eq!(bytes.pop(), Some(0xc3));
    bytes
}

#[test]
fn high_index_and_base_bits_and_zero_displacement_are_encoded() {
    let address = MemoryAddress {
        base: Reg::R13,
        index: Some((Reg::R12, Scale::Eight)),
        offset: 0,
    };
    assert_eq!(
        encoded(MachineInstr::LoadAddress {
            dst: Reg::R11,
            address,
            bytes: 8
        }),
        [0x4f, 0x8b, 0x5c, 0xe5, 0]
    );
    assert_eq!(
        encoded(MachineInstr::EffectiveAddress {
            dst: Reg::R11,
            address
        }),
        [0x4f, 0x8d, 0x5c, 0xe5, 0]
    );
    assert_eq!(
        encoded(MachineInstr::StoreAddress {
            src: Reg::Rsi,
            address,
            bytes: 1
        }),
        [0x43, 0x88, 0x74, 0xe5, 0]
    );
    assert_eq!(
        encoded(MachineInstr::StoreAddressImmediate {
            address,
            value: -7,
            bytes: 2
        }),
        [0x66, 0x43, 0xc7, 0x44, 0xe5, 0, 0xf9, 0xff]
    );
}

#[test]
fn displacement_size_changes_at_signed_byte_boundaries() {
    for (offset, suffix) in [
        (i32::MIN, vec![0, 0, 0, 0x80]),
        (-129, vec![0x7f, 0xff, 0xff, 0xff]),
        (-128, vec![0x80]),
        (0, vec![]),
        (127, vec![127]),
        (128, vec![128, 0, 0, 0]),
        (i32::MAX, vec![0xff, 0xff, 0xff, 0x7f]),
    ] {
        let address = MemoryAddress {
            base: Reg::Rdi,
            index: Some((Reg::Rsi, Scale::Four)),
            offset,
        };
        let mut expected = vec![
            0x48,
            0x8b,
            if suffix.is_empty() {
                0x04
            } else if suffix.len() == 1 {
                0x44
            } else {
                0x84
            },
            0xb7,
        ];
        expected.extend(suffix);
        assert_eq!(
            encoded(MachineInstr::LoadAddress {
                dst: Reg::Rax,
                address,
                bytes: 8
            }),
            expected
        );
    }
}

#[test]
fn reserved_registers_and_unsupported_widths_are_rejected_consistently() {
    let mut function = MachineFunction::new("invalid_indexed");
    for (base, index, bytes) in [
        (Reg::Rsp, Reg::Rsi, 8),
        (Reg::Rbp, Reg::Rsi, 8),
        (Reg::Rdi, Reg::Rsp, 8),
        (Reg::Rdi, Reg::Rbp, 8),
        (Reg::Rdi, Reg::R12, 3),
    ] {
        let address = MemoryAddress {
            base,
            index: Some((index, Scale::One)),
            offset: 0,
        };
        for instruction in [
            MachineInstr::LoadAddress {
                dst: Reg::Rax,
                address,
                bytes,
            },
            MachineInstr::StoreAddress {
                src: Reg::Rax,
                address,
                bytes,
            },
            MachineInstr::StoreAddressImmediate {
                address,
                value: 9,
                bytes,
            },
        ] {
            function.instructions = vec![instruction, MachineInstr::Ret];
            assert!(function.emit_machine_code().is_err(), "{instruction:?}");
            assert!(function.emit_gnu_assembly().is_err(), "{instruction:?}");
        }
        if bytes != 3 {
            function.instructions = vec![
                MachineInstr::EffectiveAddress {
                    dst: Reg::Rax,
                    address,
                },
                MachineInstr::Ret,
            ];
            assert!(function.emit_machine_code().is_err());
            assert!(function.emit_gnu_assembly().is_err());
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn all_allocatable_base_index_pairs_match_gnu_assembly() {
    use object::{Object, ObjectSection};
    use std::process::Command;
    let mut function = MachineFunction::new("sib_matrix");
    for (b, base) in Reg::ALLOCATABLE.into_iter().enumerate() {
        for (i, index) in Reg::ALLOCATABLE.into_iter().enumerate() {
            let reg = Reg::ALLOCATABLE[(b + i) % Reg::ALLOCATABLE.len()];
            for scale in [Scale::One, Scale::Two, Scale::Four, Scale::Eight] {
                for offset in [i32::MIN, -129, -128, 0, 127, 128, i32::MAX] {
                    let address = MemoryAddress {
                        base,
                        index: Some((index, scale)),
                        offset,
                    };
                    function
                        .instructions
                        .push(MachineInstr::EffectiveAddress { dst: reg, address });
                    for bytes in [1, 2, 4, 8] {
                        function.instructions.extend([
                            MachineInstr::LoadAddress {
                                dst: reg,
                                address,
                                bytes,
                            },
                            MachineInstr::StoreAddress {
                                src: reg,
                                address,
                                bytes,
                            },
                            MachineInstr::StoreAddressImmediate {
                                address,
                                value: -7,
                                bytes,
                            },
                        ]);
                    }
                }
            }
        }
    }
    function.instructions.push(MachineInstr::Ret);
    let dir = std::env::temp_dir().join(format!(
        "kfrc-sib-assembler-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("sib.s"), function.emit_gnu_assembly().unwrap()).unwrap();
    let compiler = Command::new("cc")
        .current_dir(&dir)
        .args(["-c", "sib.s", "-o", "sib.o"])
        .output()
        .unwrap();
    assert!(
        compiler.status.success(),
        "{}",
        String::from_utf8_lossy(&compiler.stderr)
    );
    let object = std::fs::read(dir.join("sib.o")).unwrap();
    let object = object::File::parse(&*object).unwrap();
    let expected = object.section_by_name(".text").unwrap().data().unwrap();
    let actual = function.emit_machine_code().unwrap();
    assert_eq!(actual.len(), expected.len());
    if let Some(position) = actual.iter().zip(expected).position(|(a, b)| a != b) {
        panic!(
            "encoded bytes differ from GNU assembler at {position}: native={:02x?}, gas={:02x?}",
            &actual[position..actual.len().min(position + 16)],
            &expected[position..expected.len().min(position + 16)]
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
