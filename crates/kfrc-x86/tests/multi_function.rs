use kfrc_x86::machine::x86::{MachineFunction, MachineInstr, Reg};
use std::fs;
use std::path::PathBuf;
use std::process::{self, Command};
use std::sync::atomic::{AtomicU64, Ordering};

fn write_native_answer_object() -> Result<Vec<u8>, kfrc_x86::ObjectError> {
    let mut answer = MachineFunction::new("native_answer");
    answer.instructions = vec![
        MachineInstr::MovImm {
            dst: Reg::Rcx,
            value: 42,
        },
        MachineInstr::Mov {
            dst: Reg::Rax,
            src: Reg::Rcx,
        },
        MachineInstr::Ret,
    ];
    let mut main = MachineFunction::new("main");
    main.symbols.push("native_answer".into());
    main.instructions = vec![MachineInstr::Call { target: 0 }, MachineInstr::Ret];
    kfrc_x86::write_object(
        &[answer, main]
            .into_iter()
            .map(|function| kfrc_x86::FunctionCode {
                function,
                binding: kfrc_x86::SymbolBinding::Global { hidden: false },
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn native_answer_object_has_correct_code_symbols_and_plt32_relocation() {
    use object::{Object, ObjectSection, ObjectSymbol, RelocationTarget};
    let bytes = write_native_answer_object().unwrap();
    assert_eq!(&bytes[..4], b"\x7fELF");
    let object = object::File::parse(bytes.as_slice()).unwrap();
    assert_eq!(object.architecture(), object::Architecture::X86_64);
    assert_eq!(object.kind(), object::ObjectKind::Relocatable);
    let text = object.section_by_name(".text").unwrap();
    let code = text.data().unwrap();
    let expected_answer = [0xb9, 42, 0, 0, 0, 0x48, 0x89, 0xc8, 0xc3];
    let expected_main = [0x55, 0x48, 0x89, 0xe5, 0xe8, 0, 0, 0, 0, 0xc9, 0xc3];
    let answer = object
        .symbols()
        .find(|s| s.name() == Ok("native_answer"))
        .unwrap();
    let main = object.symbols().find(|s| s.name() == Ok("main")).unwrap();
    for (symbol, expected) in [
        (&answer, expected_answer.as_slice()),
        (&main, expected_main.as_slice()),
    ] {
        assert_eq!(symbol.kind(), object::SymbolKind::Text);
        assert!(symbol.is_global());
        assert_eq!(symbol.section_index(), Some(text.index()));
        assert_eq!(symbol.size(), expected.len() as u64);
        assert_eq!(
            &code[symbol.address() as usize..symbol.address() as usize + expected.len()],
            expected
        );
    }
    let relocs: Vec<_> = text.relocations().collect();
    assert_eq!(relocs.len(), 1);
    let (offset, relocation) = &relocs[0];
    assert_eq!(*offset, main.address() + 5);
    assert_eq!(relocation.addend(), -4);
    assert_eq!(
        relocation.flags(),
        object::RelocationFlags::Elf {
            r_type: object::elf::R_X86_64_PLT32
        }
    );
    let RelocationTarget::Symbol(target) = relocation.target() else {
        panic!("expected function symbol")
    };
    assert_eq!(
        object.symbol_by_index(target).unwrap().name().unwrap(),
        "native_answer"
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
