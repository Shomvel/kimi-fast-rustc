//! Private constants are object-local; real statics remain linkable across CGUs.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::process::Command;

use kfrc_body::{DataBinding, DataObject, DataRelocation};
use kfrc_x86::{FunctionCode, ObjectError, SymbolBinding, write_object_with_data};
use kfrc_x86::machine::x86::{MachineFunction, MachineInstr, Reg};

fn read(symbol: &str, target: &str, indirections: usize) -> FunctionCode {
    let mut function = MachineFunction::new(symbol);
    function.symbols = vec![target.into()];
    function.instructions.push(MachineInstr::SymbolAddress {
        dst: Reg::Rax,
        symbol: 0,
        addend: 0,
    });
    for _ in 0..indirections {
        function.instructions.push(MachineInstr::LoadIndirect {
            dst: Reg::Rax,
            address: Reg::Rax,
            offset: 0,
            bytes: 8,
        });
    }
    function.instructions.push(MachineInstr::Ret);
    FunctionCode {
        function,
        binding: SymbolBinding::Global { hidden: false },
    }
}

fn local_constants(value: u64) -> Vec<DataObject> {
    vec![
        DataObject {
            symbol: ".Lkfrc_alloc0".into(),
            bytes: value.to_le_bytes().to_vec(),
            align: 8,
            binding: DataBinding::Local,
            writable: false,
            thread_local: false,
            unnamed_address: false,
            relocations: vec![],
        },
        DataObject {
            symbol: ".Lkfrc_alloc1".into(),
            bytes: vec![0; 8],
            align: 8,
            binding: DataBinding::Local,
            writable: false,
            thread_local: false,
            unnamed_address: false,
            relocations: vec![DataRelocation {
                offset: 0,
                target: ".Lkfrc_alloc0".into(),
                addend: 0,
            }],
        },
    ]
}

#[test]
fn repeated_private_constants_link_independently_and_global_statics_cross_objects() {
    let mut first_data = local_constants(17);
    first_data.extend([
        DataObject {
            symbol: "shared_static".into(),
            bytes: 41_u64.to_le_bytes().to_vec(),
            align: 8,
            binding: DataBinding::Global { hidden: true },
            writable: true,
            thread_local: false,
            unnamed_address: false,
            relocations: vec![],
        },
        DataObject {
            symbol: "visible_static".into(),
            bytes: 53_u64.to_le_bytes().to_vec(),
            align: 8,
            binding: DataBinding::Global { hidden: false },
            writable: false,
            thread_local: false,
            unnamed_address: false,
            relocations: vec![],
        },
    ]);
    let first =
        write_object_with_data(&[read("private_first", ".Lkfrc_alloc1", 2)], &first_data).unwrap();
    let second = write_object_with_data(
        &[
            read("private_second", ".Lkfrc_alloc1", 2),
            read("cross_static", "shared_static", 1),
            read("cross_visible", "visible_static", 1),
        ],
        &local_constants(29),
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "kfrc-data-binding-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("first.o"), first).unwrap();
    std::fs::write(dir.join("second.o"), second).unwrap();
    for input in ["first.o", "second.o"] {
        let symbols = Command::new("readelf")
            .current_dir(&dir)
            .args(["-sW", "-rW", input])
            .output()
            .unwrap();
        assert!(symbols.status.success());
        std::fs::write(dir.join(format!("{input}.readelf.txt")), &symbols.stdout).unwrap();
        let symbols = String::from_utf8_lossy(&symbols.stdout);
        for name in [".Lkfrc_alloc0", ".Lkfrc_alloc1"] {
            assert!(
                symbols.lines().any(|line| line.contains("OBJECT")
                    && line.contains("LOCAL")
                    && line.ends_with(name)),
                "{symbols}"
            );
            assert!(
                !symbols
                    .lines()
                    .any(|line| line.contains("GLOBAL") && line.ends_with(name)),
                "{symbols}"
            );
        }
        if input == "first.o" {
            assert!(
                symbols.lines().any(|line| line.contains("GLOBAL")
                    && line.contains("HIDDEN")
                    && line.ends_with("shared_static")),
                "{symbols}"
            );
            assert!(
                symbols.lines().any(|line| line.contains("GLOBAL")
                    && line.contains("DEFAULT")
                    && line.ends_with("visible_static")),
                "{symbols}"
            );
        } else {
            assert!(
                symbols.lines().any(|line| line.contains("GLOBAL")
                    && line.contains("UND")
                    && line.ends_with("shared_static")),
                "{symbols}"
            );
        }
    }
    std::fs::write(dir.join("driver.c"), r#"
#include <stdint.h>
#include <stdio.h>
extern uint64_t private_first(void),private_second(void),cross_static(void),cross_visible(void);
extern uint64_t shared_static;
int main(void) {
    if (private_first()!=17 || private_second()!=29 || cross_static()!=41 || cross_visible()!=53) return 1;
    shared_static=73;
    if (cross_static()!=73 || private_first()!=17 || private_second()!=29) return 2;
    puts("private constants/cross-object static binding PASS");
    return 0;
}

"#).unwrap();
    let link = Command::new("cc")
        .current_dir(&dir)
        .args(["driver.c", "first.o", "second.o", "-o", "binding"])
        .output()
        .unwrap();
    std::fs::write(dir.join("link.stderr"), &link.stderr).unwrap();
    assert!(
        link.status.success(),
        "{}",
        String::from_utf8_lossy(&link.stderr)
    );
    let run = Command::new(dir.join("binding")).output().unwrap();
    std::fs::write(dir.join("run.stdout"), &run.stdout).unwrap();
    std::fs::write(dir.join("run.stderr"), &run.stderr).unwrap();
    assert!(
        run.status.success(),
        "{:?}: {}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("PASS"));
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

#[test]
fn one_object_rejects_duplicate_definitions_including_conflicting_bindings() {
    let mut data = local_constants(17);
    let mut conflicting = data[0].clone();
    conflicting.binding = DataBinding::Global { hidden: true };
    data.push(conflicting);
    assert!(matches!(
        write_object_with_data(&[], &data),
        Err(ObjectError::DuplicateSymbol(_))
    ));
    assert!(matches!(
        write_object_with_data(&[read("same", "target", 1), read("same", "target", 1)], &[]),
        Err(ObjectError::DuplicateSymbol(_))
    ));
    let mut data = local_constants(17);
    data[0].symbol = "function".into();
    assert!(matches!(
        write_object_with_data(&[read("function", "target", 1)], &data),
        Err(ObjectError::DuplicateSymbol(_))
    ));
}
