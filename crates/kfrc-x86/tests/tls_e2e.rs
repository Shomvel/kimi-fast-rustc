//! Local-exec ELF TLS is storage owned by each Linux thread.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::process::Command;

use kfrc_body::*;
use kfrc_son::{BuildError, build_graph};
use kfrc_x86::{FunctionCode, ObjectError, SymbolBinding, write_object_with_data};
use kfrc_x86::machine::x86::{MachineFunction, MachineInstr, Reg};

fn address_function(symbol: &str) -> FunctionCode {
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
        format!("address_{symbol}"),
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let result = body.add_local(LocalDecl {
        ty: pointer,
        mutability: Mutability::Mut,
        name: None,
    });
    body.push_statement(StatementKind::Assign(
        Place::local(result),
        Rvalue::ThreadLocalRef {
            symbol: symbol.into(),
        },
    ));
    body.finish_block(Terminator::Return, false);
    assert!(
        matches!(build_graph(&body.finish()), Err(BuildError::Unsupported { what, .. })
        if what.contains("TLS") || what.contains("thread-local") || what.contains("ThreadLocal"))
    );
    // Machine encoding and ELF TLS remain independently tested. This does not
    // grant TLS support to the semantic Rust input pipeline.
    let mut function = MachineFunction::new(format!("address_{symbol}"));
    function.symbols = vec![symbol.into()];
    function.instructions = vec![
        MachineInstr::ThreadLocalAddress {
            dst: Reg::Rax,
            symbol: 0,
        },
        MachineInstr::Ret,
    ];
    FunctionCode {
        function,
        binding: SymbolBinding::Global { hidden: false },
    }
}

fn datum(symbol: &str, value: u64, writable: bool, thread_local: bool) -> DataObject {
    DataObject {
        binding: DataBinding::Global { hidden: true },
        symbol: symbol.into(),
        bytes: value.to_le_bytes().to_vec(),
        align: 64,
        writable,
        thread_local,
        unnamed_address: false,
        relocations: vec![],
    }
}

#[test]
fn tls_templates_external_symbols_and_thread_isolation_run_in_both_emitters() {
    let mut functions: Vec<_> = [
        "tls_init",
        "tls_zero",
        "tls_readonly",
        "tls_pointer",
        "tls_external",
    ]
    .into_iter()
    .map(address_function)
    .collect();
    // Exercise both REX.R/B and the SIB base encoding, with saved-register restoration.
    for reg in [Reg::R8, Reg::R12] {
        let mut function = MachineFunction::new(format!("address_extended_{reg}"));
        function.symbols = vec!["tls_init".into()];
        if reg == Reg::R12 {
            function.callee_saved = vec![reg];
        }
        function.instructions = vec![
            MachineInstr::ThreadLocalAddress {
                dst: reg,
                symbol: 0,
            },
            MachineInstr::Mov {
                dst: Reg::Rax,
                src: reg,
            },
            MachineInstr::Ret,
        ];
        functions.push(FunctionCode {
            function,
            binding: SymbolBinding::Global { hidden: false },
        });
    }
    let mut pointer = datum("tls_pointer", 0, true, true);
    pointer.relocations.push(DataRelocation {
        offset: 0,
        target: "readonly_words".into(),
        addend: 8,
    });
    let mut readonly = datum("readonly_words", 17, false, false);
    readonly.bytes.extend_from_slice(&29_u64.to_le_bytes());
    let data = [
        datum("tls_init", 41, true, true),
        datum("tls_zero", 0, true, true),
        datum("tls_readonly", 53, false, true),
        pointer,
        readonly,
    ];
    let bytes = write_object_with_data(&functions, &data).unwrap();
    let templates = write_object_with_data(&[], &data).unwrap();
    let assembly = functions
        .iter()
        .map(|f| f.function.emit_gnu_assembly().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let dir = std::env::temp_dir().join(format!(
        "kfrc-tls-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("tls.o"), bytes).unwrap();
    std::fs::write(dir.join("templates.o"), templates).unwrap();
    std::fs::write(dir.join("tls.s"), assembly).unwrap();
    std::fs::write(dir.join("driver.c"), r#"
#define _GNU_SOURCE
#include <stdint.h>
#include <pthread.h>
#include <stdio.h>
#include <signal.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>
__thread uint64_t tls_external = 67;
extern uint64_t *address_tls_init(void), *address_tls_zero(void), *address_tls_readonly(void);
extern uint64_t **address_tls_pointer(void), *address_tls_external(void);
extern uint64_t *address_extended_r8(void), *address_extended_r12(void);
extern const uint64_t readonly_words[2];
static pthread_barrier_t barrier;
static uintptr_t addresses[5];
static int verify(unsigned index) {
    uint64_t *init = address_tls_init(), *zero = address_tls_zero(), *ro = address_tls_readonly();
    int error = 0;
    if (((uintptr_t)init % 64) || *init != 41 || *zero != 0 || *ro != 53) error = 1;
    if (*address_tls_pointer() != readonly_words + 1 || **address_tls_pointer() != 29) error = 2;
    if (address_tls_external() != &tls_external || tls_external != 67) error = 3;
    if (address_extended_r8() != init || address_extended_r12() != init) error = 4;
    *init = 1000+index; *zero = 2000+index; tls_external = 3000+index;
    addresses[index] = (uintptr_t)init;
    pthread_barrier_wait(&barrier);
    if (error) return error;
    for (unsigned j=0;j<5;j++) if (j!=index && addresses[j]==addresses[index]) return 5;
    for (unsigned j=0;j<1000;j++) {
        if (address_tls_init()!=init || *init!=1000+index || *zero!=2000+index ||
            tls_external!=3000+index || *ro!=53) return 6;
    }
    return 0;
}
static void *worker(void *argument) { return (void *)(uintptr_t)verify((unsigned)(uintptr_t)argument); }
static void *fresh(void *argument) {
    (void)argument;
    return (void *)(uintptr_t)(*address_tls_init()!=41 || *address_tls_zero()!=0 || tls_external!=67);
}
int main(void) {
    pthread_t threads[4];
    if (pthread_barrier_init(&barrier,0,5)) return 10;
    for (unsigned i=0;i<4;i++) if (pthread_create(&threads[i],0,worker,(void *)(uintptr_t)(i+1))) return 11;
    if (verify(0)) return 12;
    for (unsigned i=0;i<4;i++) { void *status; if (pthread_join(threads[i],&status) || status) return 13; }
    pthread_barrier_destroy(&barrier);
    for (unsigned i=0;i<4;i++) {
        void *status;
        if (pthread_create(&threads[0],0,fresh,0) || pthread_join(threads[0],&status) || status) return 14;
    }
    pid_t child = fork();
    if (child<0) return 15;
    if (!child) {
        struct rlimit limit = {0,0}; setrlimit(RLIMIT_CORE,&limit);
        *(volatile uint64_t *)readonly_words=99; _exit(1);
    }
    int status;
    if (waitpid(child,&status,0)!=child || !WIFSIGNALED(status) || WTERMSIG(status)!=SIGSEGV) return 16;
    puts("ELF TLS initialization/external/isolation/relocation PASS");
    return 0;
}
"#).unwrap();
    let headers = Command::new("readelf")
        .current_dir(&dir)
        .args(["-SW", "-sW", "-rW", "tls.o"])
        .output()
        .unwrap();
    assert!(headers.status.success());
    std::fs::write(dir.join("object.readelf.txt"), &headers.stdout).unwrap();
    let headers = String::from_utf8_lossy(&headers.stdout);
    assert!(
        headers.contains(".tdata") && headers.contains(".tbss") && headers.contains("TPOFF32"),
        "{headers}"
    );
    assert!(
        headers.lines().any(|line| line.contains("TLS")
            && line.contains("UND")
            && line.contains("tls_external")),
        "{headers}"
    );
    for (inputs, executable) in [
        (vec!["tls.o"], "encoded"),
        (vec!["tls.s", "templates.o"], "assembled"),
    ] {
        let link = Command::new("cc")
            .current_dir(&dir)
            .arg("driver.c")
            .args(inputs)
            .args(["-pthread", "-O1", "-o", executable])
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
        let program = Command::new("readelf")
            .current_dir(&dir)
            .args(["-lW", executable])
            .output()
            .unwrap();
        std::fs::write(
            dir.join(format!("{executable}.readelf.txt")),
            &program.stdout,
        )
        .unwrap();
        assert!(
            program.status.success() && String::from_utf8_lossy(&program.stdout).contains(" TLS ")
        );
    }
    // A local-exec object must never silently acquire process-global semantics in a DSO.
    let shared = Command::new("cc")
        .current_dir(&dir)
        .args(["-shared", "tls.o", "-o", "unsupported.so"])
        .output()
        .unwrap();
    assert!(!shared.status.success());
    assert!(
        String::from_utf8_lossy(&shared.stderr).contains("TPOFF32"),
        "{}",
        String::from_utf8_lossy(&shared.stderr)
    );
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
fn object_writer_rejects_mixing_tls_with_ordinary_addresses() {
    let mut ordinary = address_function("same_symbol");
    let tls = datum("same_symbol", 41, true, true);
    for instruction in &mut ordinary.function.instructions {
        if let MachineInstr::ThreadLocalAddress { dst, symbol } = *instruction {
            *instruction = MachineInstr::SymbolAddress {
                dst,
                symbol,
                addend: 0,
            };
        }
    }
    assert!(matches!(
        write_object_with_data(&[ordinary], &[tls]),
        Err(ObjectError::SymbolKindMismatch { .. })
    ));
    assert!(matches!(
        write_object_with_data(
            &[address_function("same_symbol")],
            &[datum("same_symbol", 0, true, false)]
        ),
        Err(ObjectError::SymbolKindMismatch { .. })
    ));
    let mut pointer = datum("ordinary_pointer", 0, true, false);
    pointer.relocations.push(DataRelocation {
        offset: 0,
        target: "same_symbol".into(),
        addend: 0,
    });
    assert!(matches!(
        write_object_with_data(&[], &[datum("same_symbol", 0, true, true), pointer]),
        Err(ObjectError::SymbolKindMismatch { .. })
    ));
    let mut mixed = address_function("external_symbol");
    mixed.function.instructions.insert(
        0,
        MachineInstr::SymbolAddress {
            dst: Reg::Rax,
            symbol: 0,
            addend: 0,
        },
    );
    assert!(matches!(
        write_object_with_data(&[mixed], &[]),
        Err(ObjectError::SymbolKindMismatch { .. })
    ));
}
