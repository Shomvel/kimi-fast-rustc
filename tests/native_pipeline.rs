use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use kimi_fast_rustc::{
    frontend, interpreter, native, native_cli, optimizer_pipeline, pipelined_native, placement,
    son_optimize,
};

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        for attempt in 0..128 {
            let path = std::env::temp_dir().join(format!(
                "kfrc-native-pipeline-test-{}-{timestamp:x}-{sequence}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("failed to create test directory: {error}"),
            }
        }
        panic!("failed to allocate a unique test directory")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn native_cli_emits_assembly_for_optimized_constant_main() {
    let directory = TestDirectory::new();
    let input = directory.path.join("input.rs");
    let output = directory.path.join("output.s");
    fs::write(&input, "fn main() -> i64 { return 40 + 2; }\n").unwrap();

    let returned = native_cli::run_with_args(vec![
        input.into_os_string(),
        "--emit=asm".into(),
        "-o".into(),
        output.clone().into_os_string(),
    ])
    .unwrap();

    assert_eq!(returned, output);
    let assembly = fs::read_to_string(output).unwrap();
    assert!(assembly.starts_with(".intel_syntax noprefix\n.text\n.globl main\n"));
    assert!(assembly.contains("\tmov rcx, 42\n"), "{assembly}");
    assert!(assembly.contains("\tmov rax, rcx\n"), "{assembly}");
    assert!(!assembly.contains("\tadd"), "{assembly}");
    assert!(assembly.contains("\tret"), "{assembly}");
    assert!(!assembly.contains("llvm"));
}

#[test]
fn native_cli_emits_value_numbered_repeated_expressions_as_assembly_and_object() {
    let directory = TestDirectory::new();
    let input = directory.path.join("input.rs");
    let assembly_output = directory.path.join("output.s");
    let object_output = directory.path.join("output.o");
    let source = "fn main(value: i64) -> i64 { let first = value * value; let second = value * value; return first + second; }";
    fs::write(&input, source).unwrap();

    let mut optimized = frontend::parse(source).unwrap();
    let stats = optimizer_pipeline::optimize(&mut optimized);
    assert_eq!(stats.value_numbering_merges, 1);
    let placed = placement::place_graph(&optimized).unwrap();
    let expected_assembly = native::compile_graph_to_assembly(&placed, "main").unwrap();
    let expected_code = native::compile_graph(&placed, "main")
        .unwrap()
        .emit_machine_code()
        .unwrap();

    let returned = native_cli::run_with_args(vec![
        input.clone().into_os_string(),
        "--emit=asm".into(),
        "-o".into(),
        assembly_output.clone().into_os_string(),
    ])
    .unwrap();
    assert_eq!(returned, assembly_output);
    let assembly = fs::read_to_string(assembly_output).unwrap();
    assert_eq!(assembly, expected_assembly);
    assert_eq!(assembly.matches("\timul ").count(), 1, "{assembly}");
    assert_eq!(assembly.matches("\tadd ").count(), 1, "{assembly}");
    assert!(assembly.contains("\tret"), "{assembly}");

    let returned = native_cli::run_with_args(vec![
        input.into_os_string(),
        "--emit=obj".into(),
        "-o".into(),
        object_output.clone().into_os_string(),
    ])
    .unwrap();
    assert_eq!(returned, object_output);
    let object = fs::read(object_output).unwrap();
    assert_eq!(&object[..4], b"\x7fELF");
    assert_eq!(&object[16..18], &[1, 0], "object must be relocatable");
    assert_eq!(&object[18..20], &[62, 0], "object must target x86_64");
    assert_eq!(&object[64..64 + expected_code.len()], expected_code);
}

#[test]
fn optimized_graph_is_placed_and_compiled_without_llvm() {
    let mut graph = frontend::parse("fn main() -> i64 { return 40 + 2; }").unwrap();
    let stats = son_optimize::optimize(&mut graph);
    assert!(stats.rewrites() > 0);

    let placed = placement::place_graph(&graph).unwrap();
    let assembly = native::compile_graph_to_assembly(&placed, "main").unwrap();
    assert!(assembly.contains("main:"));
    assert!(assembly.contains("mov rcx, 42"));
    assert!(!assembly.contains("llvm"));
}

#[test]
fn optimization_preserves_interpreted_results() {
    let source = "fn main(value: i64) -> i64 { let left = value + 0; return left * 1; }";
    let original = frontend::parse(source).unwrap();
    let mut optimized = original.clone();
    son_optimize::optimize(&mut optimized);
    let placed = placement::place_graph(&optimized).unwrap();

    for argument in [i64::MIN, -1, 0, 1, 41, i64::MAX] {
        let expected = interpreter::interpret(&original, &[argument]).unwrap();
        assert_eq!(interpreter::interpret(&placed, &[argument]), Ok(expected));
    }
}

#[test]
fn pipelined_sources_preserve_order_and_direct_object_bytes() {
    let inputs = vec![
        (
            "constant".to_owned(),
            "fn main() -> i64 { return 40 + 2; }".to_owned(),
        ),
        (
            "arithmetic".to_owned(),
            "fn main(x: i64) -> i64 { let square = x * x; return square + x * x; }".to_owned(),
        ),
        (
            "comparison".to_owned(),
            "fn main(x: i64) -> bool { return x < 7; }".to_owned(),
        ),
        (
            "control_flow".to_owned(),
            "fn main(x: i64) -> i64 { if x < 0 { return -x; } return x + 1; }".to_owned(),
        ),
    ];
    let expected: Vec<_> = inputs
        .iter()
        .map(|(name, source)| {
            let mut graph = frontend::parse(source).unwrap();
            optimizer_pipeline::optimize(&mut graph);
            let graph = placement::place_for_native(&graph).unwrap();
            native::compile_graph(&graph, name.clone())
                .unwrap()
                .emit_object()
                .unwrap()
        })
        .collect();

    let functions = pipelined_native::compile_sources(inputs, 2).unwrap();
    let names: Vec<_> = functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    let objects: Vec<_> = functions
        .iter()
        .map(|function| function.emit_object().unwrap())
        .collect();

    assert_eq!(
        names,
        ["constant", "arithmetic", "comparison", "control_flow"]
    );
    assert_eq!(objects, expected);
}
