//! Target sections retain writable state and protect immutable relocated data.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::process::Command;

use kfrc_body::*;
use kfrc_son::{build_graph, lower_layout, verify_graph};
use kfrc_x86::{FunctionCode, SymbolBinding, lower_function, write_object_with_data};

fn exchange_function() -> FunctionCode {
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
    let mut body = BodyBuilder::new(
        "exchange_static",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let mut local = |ty| {
        body.add_local(LocalDecl {
            ty,
            mutability: Mutability::Mut,
            name: None,
        })
    };
    let result = local(word);
    let argument = local(word);
    let address = local(pointer);
    body.set_arg_count(1);
    let constant = body.add_constant(Constant {
        ty: pointer,
        kind: ConstantKind::Address {
            symbol: "writable_word".into(),
            addend: 0,
        },
    });
    body.push_statement(StatementKind::Assign(
        Place::local(address),
        Rvalue::Use(Operand::Constant(constant)),
    ));
    let value = Place {
        local: address,
        projection: body.add_projection(vec![ProjectionElem::Deref]),
    };
    body.push_statement(StatementKind::Assign(
        Place::local(result),
        Rvalue::Use(Operand::Copy(value)),
    ));
    body.push_statement(StatementKind::Assign(
        value,
        Rvalue::Use(Operand::Copy(Place::local(argument))),
    ));
    body.finish_block(Terminator::Return, false);
    let body = body.finish();
    let mut graph = build_graph(&body).unwrap();
    verify_graph(&mut graph).unwrap();
    lower_layout(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    FunctionCode {
        function: lower_function(&graph).unwrap(),
        binding: SymbolBinding::Global { hidden: false },
    }
}

#[test]
fn encoded_writable_data_and_relocations_survive_mutation_while_relro_is_protected() {
    let data = vec![
        DataObject {
            binding: DataBinding::Global { hidden: true },
            symbol: "writable_word".into(),
            bytes: 41_u64.to_le_bytes().to_vec(),
            align: 8,
            thread_local: false,
            writable: true,
            relocations: vec![],
        },
        DataObject {
            binding: DataBinding::Global { hidden: true },
            symbol: "writable_zero".into(),
            bytes: vec![0; 8],
            align: 8,
            thread_local: false,
            writable: true,
            relocations: vec![],
        },
        DataObject {
            binding: DataBinding::Global { hidden: true },
            symbol: "readonly_words".into(),
            bytes: [17_u64.to_le_bytes(), 29_u64.to_le_bytes()].concat(),
            align: 16,
            thread_local: false,
            writable: false,
            relocations: vec![],
        },
        DataObject {
            binding: DataBinding::Global { hidden: true },
            symbol: "writable_pointer".into(),
            bytes: vec![0; 8],
            align: 8,
            thread_local: false,
            writable: true,
            relocations: vec![DataRelocation {
                offset: 0,
                target: "readonly_words".into(),
                addend: 8,
            }],
        },
        DataObject {
            binding: DataBinding::Global { hidden: true },
            symbol: "readonly_pointer".into(),
            bytes: vec![0; 8],
            align: 8,
            thread_local: false,
            writable: false,
            relocations: vec![DataRelocation {
                offset: 0,
                target: "writable_word".into(),
                addend: 0,
            }],
        },
    ];
    let object = write_object_with_data(&[exchange_function()], &data).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "kfrc-writable-data-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("data.o"), object).unwrap();
    std::fs::write(
        dir.join("driver.c"),
        r#"
#include <stdint.h>
#include <signal.h>
#include <stdio.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>
extern uint64_t writable_word, writable_zero;
extern const uint64_t readonly_words[2];
extern const uint64_t *writable_pointer;
extern uint64_t * const readonly_pointer;
extern uint64_t exchange_static(uint64_t);
static int protected_from_writes(const void *address) {
    pid_t child = fork();
    if (child < 0) return 0;
    if (child == 0) {
        struct rlimit limit = {0, 0};
        setrlimit(RLIMIT_CORE, &limit);
        volatile unsigned char *byte = (volatile unsigned char *)address;
        *byte ^= 1;
        _exit(0);
    }
    int status;
    if (waitpid(child, &status, 0) != child) return 0;
    return WIFSIGNALED(status) && WTERMSIG(status) == SIGSEGV;
}
int main(void) {
    if (writable_word != 41 || writable_zero != 0) return 1;
    if (writable_pointer != readonly_words + 1 || *writable_pointer != 29) return 2;
    if (readonly_pointer != &writable_word) return 3;
    if (exchange_static(73) != 41 || writable_word != 73) return 4;
    *readonly_pointer = 89;
    if (exchange_static(101) != 89 || writable_word != 101) return 5;
    writable_zero = 313;
    writable_pointer = readonly_words;
    if (writable_zero != 313 || *writable_pointer != 17) return 6;
    if (!protected_from_writes(readonly_words)) return 7;
    if (!protected_from_writes(&readonly_pointer)) return 8;
    puts("writable data/relocations/read-only protection PASS");
    return 0;
}
"#,
    )
    .unwrap();
    let link = Command::new("cc")
        .current_dir(&dir)
        .args([
            "driver.c",
            "data.o",
            "-fPIE",
            "-pie",
            "-Wl,-z,relro,-z,now",
            "-o",
            "driver",
        ])
        .output()
        .unwrap();
    assert!(
        link.status.success(),
        "{}",
        String::from_utf8_lossy(&link.stderr)
    );
    let run = Command::new(dir.join("driver")).output().unwrap();
    assert!(
        run.status.success(),
        "status {:?}: {}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("PASS"));
    std::fs::remove_dir_all(dir).unwrap();
}
