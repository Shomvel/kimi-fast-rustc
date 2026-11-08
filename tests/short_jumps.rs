use kimi_fast_rustc::x86::{Condition, MachineFunction, MachineInstr, Reg};
use kimi_fast_rustc::x86_encode::{CallRelocation, encode_with_relocations};

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
            let offset = 4 + body_size;

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
                assert_eq!(code[4], opcode, "{kind:?}");
                assert_eq!(code[5], 0x7f, "{kind:?}");
            } else {
                let displacement_offset = match kind {
                    Branch::Jmp => {
                        assert_eq!(code[4], 0xe9);
                        5
                    }
                    Branch::Jcc => {
                        assert_eq!(&code[4..6], &[0x0f, 0x84]);
                        6
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
    assert_eq!(code[4..6], [0xeb, 125]);
    assert_eq!(code[129..131], [0xeb, 0]);
    assert_eq!(code.len(), 133);

    let mut instructions = vec![MachineInstr::Jmp(2)];
    instructions.extend(padding(122));
    instructions.extend([
        branch(Branch::Jcc, 3),
        MachineInstr::Label(3),
        MachineInstr::Label(2),
        MachineInstr::Ret,
    ]);
    let code = function(instructions).emit_machine_code().unwrap();
    assert_eq!(code[4..6], [0xeb, 124]);
    assert_eq!(code[128..130], [0x74, 0]);
    assert_eq!(code.len(), 132);
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

        assert_eq!(code[4], 0xe9, "{kind:?}");
        assert_eq!(read_i32(&code, 5), 132, "{kind:?}");
        let opcode = match kind {
            Branch::Jmp => 0xeb,
            Branch::Jcc => 0x74,
        };
        assert_eq!(code[9..11], [opcode, 0], "{kind:?}");
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
    let object = kimi_fast_rustc::native_program::NativeProgram {
        functions: vec![function, first, second],
    }
    .emit_object()
    .unwrap();
    let read_u64 =
        |offset: usize| u64::from_le_bytes(object[offset..offset + 8].try_into().unwrap()) as usize;
    let section_headers = read_u64(40);
    let text = section_headers + 64;
    assert_eq!(read_u64(text + 32), 32);
    let relocation = section_headers + 2 * 64;
    let relocation_offset = read_u64(relocation + 24);
    assert_eq!(read_u64(relocation + 32), 2 * 24);
    for (index, offset) in [7, 14].into_iter().enumerate() {
        let entry = relocation_offset + index * 24;
        assert_eq!(read_u64(entry), offset);
        assert_eq!(
            read_u64(entry + 8) as u32,
            kimi_fast_rustc::elf_reloc::R_X86_64_PLT32
        );
        assert_eq!(read_u64(entry + 16), -4_i64 as usize);
    }
}
