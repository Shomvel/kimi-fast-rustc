use kfrc_x86::machine::x86::{Condition, MachineFunction, MachineInstr, Reg};
use kfrc_x86::machine::x86_encode::{CallRelocation, encode_with_relocations};

#[derive(Clone, Copy, Debug)]
enum Branch {
    Jmp,
    Jcc,
}

fn function(instructions: Vec<MachineInstr>) -> MachineFunction {
    let mut function = MachineFunction::new("short_jumps");
    function.instructions = instructions;
    function
}

fn branch(kind: Branch, target: u32) -> MachineInstr {
    match kind {
        Branch::Jmp => MachineInstr::Jmp(target),
        Branch::Jcc => MachineInstr::Jcc {
            condition: Condition::Eq,
            target,
        },
    }
}

fn padding(mut size: usize) -> Vec<MachineInstr> {
    let mut instructions = Vec::new();
    while size > 3 {
        instructions.push(MachineInstr::Cqo);
        size -= 2;
    }
    match size {
        0 => {}
        2 => instructions.push(MachineInstr::Cqo),
        3 => instructions.push(MachineInstr::Mov {
            dst: Reg::Rax,
            src: Reg::Rax,
        }),
        _ => panic!("cannot create {size}-byte padding"),
    }
    instructions
}

fn read_i32(code: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(code[offset..offset + 4].try_into().unwrap())
}

#[test]
fn backward_branches_accept_minus_128_and_keep_minus_129_near() {
    for kind in [Branch::Jmp, Branch::Jcc] {
        for body_size in [126, 127] {
            let mut instructions = vec![MachineInstr::Label(1)];
            instructions.extend(padding(body_size));
            instructions.extend([branch(kind, 1), MachineInstr::Ret]);
            let code = function(instructions).emit_machine_code().unwrap();
            let offset = body_size;

            if body_size == 126 {
                let opcode = match kind {
                    Branch::Jmp => 0xeb,
                    Branch::Jcc => 0x74,
                };
                assert_eq!(code[offset], opcode, "{kind:?}");
                assert_eq!(code[offset + 1], 0x80, "{kind:?}");
            } else {
                let (long_size, displacement_offset) = match kind {
                    Branch::Jmp => {
                        assert_eq!(code[offset], 0xe9);
                        (5, offset + 1)
                    }
                    Branch::Jcc => {
                        assert_eq!(&code[offset..offset + 2], &[0x0f, 0x84]);
                        (6, offset + 2)
                    }
                };
                assert_eq!(
                    read_i32(&code, displacement_offset),
                    -(body_size as i32) - long_size,
                    "{kind:?}"
                );
            }
        }
    }
}

#[test]
fn forward_branches_accept_127_and_keep_128_near() {
    for kind in [Branch::Jmp, Branch::Jcc] {
        for body_size in [127, 128] {
            let mut instructions = vec![branch(kind, 1)];
            instructions.extend(padding(body_size));
            instructions.extend([MachineInstr::Label(1), MachineInstr::Ret]);
            let code = function(instructions).emit_machine_code().unwrap();

            if body_size == 127 {
                let opcode = match kind {
                    Branch::Jmp => 0xeb,
                    Branch::Jcc => 0x74,
                };
                assert_eq!(code[0], opcode, "{kind:?}");
                assert_eq!(code[1], 0x7f, "{kind:?}");
            } else {
                let displacement_offset = match kind {
                    Branch::Jmp => {
                        assert_eq!(code[0], 0xe9);
                        1
                    }
                    Branch::Jcc => {
                        assert_eq!(&code[..2], &[0x0f, 0x84]);
                        2
                    }
                };
                assert_eq!(read_i32(&code, displacement_offset), 128, "{kind:?}");
            }
        }
    }
}

#[test]
fn shortening_repeats_when_a_later_jump_enables_an_earlier_one() {
    let mut instructions = vec![MachineInstr::Jmp(2)];
    instructions.extend(padding(123));
    instructions.extend([
        MachineInstr::Jmp(3),
        MachineInstr::Label(3),
        MachineInstr::Label(2),
        MachineInstr::Ret,
    ]);
    let code = function(instructions).emit_machine_code().unwrap();
    assert_eq!(code[..2], [0xeb, 125]);
    assert_eq!(code[125..127], [0xeb, 0]);
    assert_eq!(code.len(), 128);

    let mut instructions = vec![MachineInstr::Jmp(2)];
    instructions.extend(padding(122));
    instructions.extend([
        branch(Branch::Jcc, 3),
        MachineInstr::Label(3),
        MachineInstr::Label(2),
        MachineInstr::Ret,
    ]);
    let code = function(instructions).emit_machine_code().unwrap();
    assert_eq!(code[..2], [0xeb, 124]);
    assert_eq!(code[124..126], [0x74, 0]);
    assert_eq!(code.len(), 127);
}

#[test]
fn near_fixups_survive_shortening_elsewhere() {
    for kind in [Branch::Jmp, Branch::Jcc] {
        let mut instructions = vec![
            MachineInstr::Jmp(2),
            branch(kind, 1),
            MachineInstr::Label(1),
        ];
        instructions.extend(padding(130));
        instructions.extend([MachineInstr::Label(2), MachineInstr::Ret]);
        let code = function(instructions).emit_machine_code().unwrap();

        assert_eq!(code[0], 0xe9, "{kind:?}");
        assert_eq!(read_i32(&code, 1), 132, "{kind:?}");
        let opcode = match kind {
            Branch::Jmp => 0xeb,
            Branch::Jcc => 0x74,
        };
        assert_eq!(code[5..7], [opcode, 0], "{kind:?}");
    }
}

fn layout_hash(code: &[u8], relocations: &[CallRelocation]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in code {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    for relocation in relocations {
        for byte in relocation
            .offset
            .to_le_bytes()
            .into_iter()
            .chain(relocation.target.bytes())
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

#[test]
fn dense_shortening_and_chains_match_known_bytes_and_call_offsets() {
    let mut adjacent = function(vec![MachineInstr::Label(0)]);
    adjacent.symbols = vec!["external".into()];
    for index in 0..128 {
        let target = index + 1;
        adjacent.instructions.push(if index % 2 == 0 {
            MachineInstr::Jmp(target)
        } else {
            branch(Branch::Jcc, target)
        });
        if index % 7 == 0 {
            adjacent.instructions.push(MachineInstr::Call { target: 0 });
        }
        adjacent.instructions.push(MachineInstr::Label(target));
    }
    adjacent.instructions.push(MachineInstr::Ret);

    let mut chain = function(Vec::new());
    chain.symbols = vec!["external".into()];
    for index in 0..64_u32 {
        let near_label = 2 * index + 1;
        let far_label = near_label + 1;
        chain.instructions.push(MachineInstr::Jmp(far_label));
        chain
            .instructions
            .extend(padding(if index % 2 == 0 { 123 } else { 122 }));
        chain.instructions.push(branch(Branch::Jcc, near_label));
        chain.instructions.extend([
            MachineInstr::Label(near_label),
            MachineInstr::Label(far_label),
        ]);
        if index % 7 == 0 {
            chain.instructions.push(MachineInstr::Call { target: 0 });
        }
    }
    chain.instructions.push(MachineInstr::Ret);

    for (function, code_len, relocation_count, expected_hash) in [
        (adjacent, 357, 19, 0x7c9a_b305_acb9_cbf7),
        (chain, 8152, 10, 0xe256_bfe2_828d_f14f),
    ] {
        let (code, relocations) = encode_with_relocations(&function).unwrap();
        assert_eq!(code.len(), code_len);
        assert_eq!(relocations.len(), relocation_count);
        assert_eq!(layout_hash(&code, &relocations), expected_hash);
    }
}

#[test]
fn labels_and_plt32_call_offsets_use_the_shortened_layout() {
    let mut function = function(vec![
        MachineInstr::Jmp(1),
        MachineInstr::Call { target: 0 },
        MachineInstr::Label(1),
        branch(Branch::Jcc, 2),
        MachineInstr::Call { target: 1 },
        MachineInstr::Label(2),
        MachineInstr::Ret,
    ]);
    function.symbols = vec!["first".into(), "second".into()];
    let (code, relocations) = encode_with_relocations(&function).unwrap();

    assert_eq!(
        code,
        [
            0x55, 0x48, 0x89, 0xe5, 0xeb, 0x05, 0xe8, 0, 0, 0, 0, 0x74, 0x05, 0xe8, 0, 0, 0, 0,
            0xc9, 0xc3,
        ]
    );
    assert_eq!(
        relocations,
        [
            CallRelocation {
                offset: 7,
                target: "first".into(),
            },
            CallRelocation {
                offset: 14,
                target: "second".into(),
            },
        ]
    );

    let mut first = MachineFunction::new("first");
    first.instructions = vec![MachineInstr::Ret];
    let mut second = MachineFunction::new("second");
    second.instructions = vec![MachineInstr::Ret];
    let functions = vec![function, first, second]
        .into_iter()
        .map(|function| kfrc_x86::FunctionCode {
            function,
            binding: kfrc_x86::SymbolBinding::Global { hidden: false },
        })
        .collect::<Vec<_>>();
    let bytes = kfrc_x86::write_object(&functions).unwrap();
    use object::{Object, ObjectSection, ObjectSymbol, RelocationTarget};
    let object = object::File::parse(bytes.as_slice()).unwrap();
    let text = object
        .sections()
        .find(|section| section.relocations().count() == 2)
        .unwrap();
    let relocs: Vec<_> = text.relocations().collect();
    assert_eq!(relocs.len(), 2);
    for ((offset, reloc), (expected_offset, name)) in
        relocs.iter().zip([(7, "first"), (14, "second")])
    {
        assert_eq!(*offset, expected_offset);
        assert_eq!(reloc.addend(), -4);
        assert_eq!(
            reloc.flags(),
            object::RelocationFlags::Elf {
                r_type: object::elf::R_X86_64_PLT32
            }
        );
        let RelocationTarget::Symbol(symbol) = reloc.target() else {
            panic!("call relocation target must be symbol")
        };
        assert_eq!(
            object.symbol_by_index(symbol).unwrap().name().unwrap(),
            name
        );
    }
}
