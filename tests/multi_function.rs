use kimi_fast_rustc::call_reloc::{CallFunction, CallRelocError, DirectCall};
use kimi_fast_rustc::multi_function::{
    MultiFunctionError, write_multi_function_object, write_native_answer_object,
};
use kimi_fast_rustc::x86::{MachineFunction, MachineInstr, Reg};
use std::fs;
use std::path::PathBuf;
use std::process::{self, Command};
use std::sync::atomic::{AtomicU64, Ordering};

const SECTION_HEADER_SIZE: usize = 64;
const SYMBOL_SIZE: usize = 24;
const RELA_TEXT_SECTION: usize = 2;
const SYMTAB_SECTION: usize = 3;
const STRTAB_SECTION: usize = 4;
const R_X86_64_PLT32: u64 = 4;

#[test]
fn native_answer_object_has_correct_code_symbols_and_plt32_relocation() {
    let object = write_native_answer_object().unwrap();
    assert_eq!(&object[..4], b"\x7fELF");
    assert_eq!(read_u16(&object, 16), 1, "ET_REL");
    assert_eq!(read_u16(&object, 18), 62, "EM_X86_64");

    let expected_native_answer = [
        0x55, 0x48, 0x89, 0xe5, 0x48, 0xc7, 0xc1, 42, 0, 0, 0, 0x48, 0x89, 0xc8, 0xc9, 0xc3,
    ];
    let expected_main = [0x55, 0x48, 0x89, 0xe5, 0xe8, 0, 0, 0, 0, 0xc9, 0xc3];
    let text = section_contents(&object, section(&object, 1));
    assert_eq!(
        text,
        [expected_native_answer.as_slice(), expected_main.as_slice()].concat()
    );

    let native_answer = symbol(&object, 1);
    assert_eq!(symbol_name(&object, native_answer), "native_answer");
    assert_eq!(native_answer[4], 0x12, "global STT_FUNC");
    assert_eq!(read_u16(native_answer, 6), 1, "defined in .text");
    assert_eq!(read_u64(native_answer, 8), 0);
    assert_eq!(
        read_u64(native_answer, 16),
        expected_native_answer.len() as u64
    );

    let main = symbol(&object, 2);
    assert_eq!(symbol_name(&object, main), "main");
    assert_eq!(main[4], 0x12, "global STT_FUNC");
    assert_eq!(read_u16(main, 6), 1, "defined in .text");
    assert_eq!(read_u64(main, 8), expected_native_answer.len() as u64);
    assert_eq!(read_u64(main, 16), expected_main.len() as u64);
    let symtab = section(&object, SYMTAB_SECTION);
    assert_eq!(read_u64(symtab, 32), 3 * SYMBOL_SIZE as u64);

    let relocations = section_contents(&object, section(&object, RELA_TEXT_SECTION));
    assert_eq!(relocations.len(), 24);
    let relocation_offset = (expected_native_answer.len() + 5) as u64;
    assert_eq!(read_u64(relocations, 0), relocation_offset);
    assert_eq!(
        read_u64(relocations, 8),
        (1_u64 << 32) | R_X86_64_PLT32,
        "relocation must target native_answer"
    );
    assert_eq!(read_i64(relocations, 16), -4);

    let return_offset = expected_native_answer.len() as i64 + 9;
    let displacement = -4 - relocation_offset as i64;
    assert_eq!(return_offset + displacement, 0);
}

#[test]
fn unsupported_abis_and_undefined_targets_are_rejected() {
    let with_argument = MachineFunction {
        name: "has_argument".into(),
        instructions: vec![
            MachineInstr::LoadArg {
                dst: Reg::Rax,
                index: 0,
            },
            MachineInstr::Ret,
        ],
        stack_slots: 0,
        callee_saved: Vec::new(),
        symbols: Vec::new(),
    };
    assert_eq!(
        write_multi_function_object(&[with_argument], &[]),
        Err(MultiFunctionError::UnsupportedNativeArguments {
            function_index: 0,
            function_name: "has_argument".into(),
            argument_index: 0,
        })
    );

    let call_with_argument = CallFunction::new(
        "main",
        vec![DirectCall {
            target: "native_answer".into(),
            argument_count: 1,
            live_caller_saved: Vec::new(),
        }],
    );
    assert_eq!(
        write_multi_function_object(&[], &[call_with_argument]),
        Err(MultiFunctionError::Call(
            CallRelocError::UnsupportedArguments {
                call_index: 0,
                argument_count: 1,
            }
        ))
    );

    let live_caller_saved = CallFunction::new(
        "main",
        vec![DirectCall {
            target: "native_answer".into(),
            argument_count: 0,
            live_caller_saved: vec![Reg::Rcx],
        }],
    );
    assert_eq!(
        write_multi_function_object(&[], &[live_caller_saved]),
        Err(MultiFunctionError::Call(
            CallRelocError::CallerSavedLiveAcrossCall {
                call_index: 0,
                reg: Reg::Rcx,
            }
        ))
    );

    let unknown_target = CallFunction::new("main", vec![DirectCall::new("missing")]);
    assert_eq!(
        write_multi_function_object(&[], &[unknown_target]),
        Err(MultiFunctionError::Call(CallRelocError::UnknownTarget {
            function_index: 0,
            call_index: 0,
            target: "missing".into(),
        }))
    );
}

#[test]
fn system_elf_tools_accept_the_self_contained_object_when_available() {
    let readelf = find_command(&["readelf", "llvm-readelf"]);
    let objdump = find_command(&["objdump", "llvm-objdump"]);
    if readelf.is_none() && objdump.is_none() {
        eprintln!("readelf/objdump not found; skipping external validation");
        return;
    }

    let directory = TempDirectory::new();
    let path = directory.0.join("native_answer.o");
    fs::write(&path, write_native_answer_object().unwrap()).unwrap();

    if let Some(readelf) = readelf {
        let output = Command::new(readelf)
            .args(["-W", "-h", "-s", "-r"])
            .arg(&path)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "{readelf} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        for expected in ["REL", "X86-64", "native_answer", "main", "R_X86_64_PLT32"] {
            assert!(
                stdout.contains(expected),
                "{readelf} output lacks {expected:?}:\n{stdout}"
            );
        }
    }

    if let Some(objdump) = objdump {
        let output = Command::new(objdump)
            .args(["-d", "-r", "-M", "intel"])
            .arg(&path)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "{objdump} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        for expected in [
            "native_answer",
            "main",
            "mov",
            "call",
            "R_X86_64_PLT32",
            "leave",
            "ret",
        ] {
            assert!(
                stdout.contains(expected),
                "{objdump} output lacks {expected:?}:\n{stdout}"
            );
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn linked_main_returns_42_on_linux_x86_64() {
    let Some(linker) = find_command(&["cc", "gcc", "clang"]) else {
        eprintln!("C linker not found; skipping linked execution");
        return;
    };
    let directory = TempDirectory::new();
    let object = directory.0.join("native_answer.o");
    let executable = directory.0.join("native_answer");
    fs::write(&object, write_native_answer_object().unwrap()).unwrap();

    let output = Command::new(linker)
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{linker} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let status = Command::new(&executable).status().unwrap();
    assert_eq!(status.code(), Some(42));
}

fn section(object: &[u8], index: usize) -> &[u8] {
    let headers = read_u64(object, 40) as usize;
    let start = headers + index * SECTION_HEADER_SIZE;
    &object[start..start + SECTION_HEADER_SIZE]
}

fn section_contents<'a>(object: &'a [u8], header: &[u8]) -> &'a [u8] {
    let offset = read_u64(header, 24) as usize;
    let size = read_u64(header, 32) as usize;
    &object[offset..offset + size]
}

fn symbol(object: &[u8], index: usize) -> &[u8] {
    let symtab = section_contents(object, section(object, SYMTAB_SECTION));
    &symtab[index * SYMBOL_SIZE..(index + 1) * SYMBOL_SIZE]
}

fn symbol_name<'a>(object: &'a [u8], symbol: &[u8]) -> &'a str {
    let strings = section_contents(object, section(object, STRTAB_SECTION));
    let start = read_u32(symbol, 0) as usize;
    let end = strings[start..]
        .iter()
        .position(|byte| *byte == 0)
        .map(|length| start + length)
        .unwrap();
    std::str::from_utf8(&strings[start..end]).unwrap()
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_i64(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn find_command<'a>(commands: &'a [&'a str]) -> Option<&'a str> {
    commands.iter().copied().find(|command| {
        Command::new(command)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = env_temp_dir().join(format!("kfrc-multi-function-test-{}-{id}", process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn env_temp_dir() -> PathBuf {
    std::env::temp_dir()
}
