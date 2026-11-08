use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use kimi_fast_rustc::{
    frontend, interpreter, native, native_batch, native_cli, native_program, optimizer_pipeline,
    pipelined_native, placement, son_optimize,
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::process::Command;

const FORWARD_SOURCE: &str =
    "fn main() -> i64 { return later(40); } fn later(value: i64) -> i64 { return value + 2; }";
const RECURSIVE_SOURCE: &str = "fn factorial(value: i64) -> i64 { if value <= 1 { return 1; } return value * factorial(value - 1); } fn main() -> i64 { return factorial(5); }";

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
        ("forward".to_owned(), FORWARD_SOURCE.to_owned()),
        ("recursive".to_owned(), RECURSIVE_SOURCE.to_owned()),
    ];
    let expected: Vec<_> = inputs
        .iter()
        .map(|(name, source)| {
            let mut graph = frontend::parse(source).unwrap();
            optimizer_pipeline::optimize(&mut graph);
            let graph = placement::place_for_native(&graph).unwrap();
            let program = if graph.functions.len() <= 1 {
                native_program::NativeProgram {
                    functions: vec![native::compile_graph(&graph, name.clone()).unwrap()],
                }
            } else {
                native_program::compile_graph(&graph).unwrap()
            };
            program.emit_object().unwrap()
        })
        .collect();

    for jobs in [1, 2, 4] {
        let programs =
            pipelined_native::compile_sources_with_jobs(inputs.clone(), 2, jobs).unwrap();
        let names: Vec<_> = programs
            .iter()
            .flat_map(|program| {
                program
                    .functions
                    .iter()
                    .map(|function| function.name.as_str())
            })
            .collect();
        let objects: Vec<_> = programs
            .iter()
            .map(|program| program.emit_object().unwrap())
            .collect();

        assert_eq!(
            names,
            [
                "constant",
                "arithmetic",
                "comparison",
                "control_flow",
                "main",
                "later",
                "factorial",
                "main",
            ]
        );
        assert_eq!(objects, expected, "object bytes differ with jobs={jobs}");
    }
}

#[test]
fn native_batch_multi_function_objects_preserve_order_calls_and_mode_bytes() {
    let directory = TestDirectory::new();
    let forward = directory.path.join("forward.kfr");
    let recursive = directory.path.join("recursive.kfr");
    fs::write(&forward, FORWARD_SOURCE).unwrap();
    fs::write(&recursive, RECURSIVE_SOURCE).unwrap();
    let inputs = [&forward, &recursive];
    let configurations = [
        (native_batch::BatchMode::Dag, 1),
        (native_batch::BatchMode::Dag, 3),
        (native_batch::BatchMode::Pipeline, 1),
        (native_batch::BatchMode::Pipeline, 3),
    ];
    let mut runs = Vec::new();
    for (index, (mode, jobs)) in configurations.into_iter().enumerate() {
        let output = directory.path.join(format!("objects-{index}"));
        let outputs = native_batch::compile_batch_with_mode(inputs, &output, jobs, mode).unwrap();
        assert_eq!(
            outputs,
            [output.join("forward.o"), output.join("recursive.o")]
        );
        runs.push(outputs);
    }

    for (index, (functions, calls)) in [
        (&["main", "later"][..], &["later"][..]),
        (&["factorial", "main"][..], &["factorial", "factorial"][..]),
    ]
    .into_iter()
    .enumerate()
    {
        let expected = fs::read(&runs[0][index]).unwrap();
        assert_object(&expected, functions, calls);
        for outputs in &runs[1..] {
            assert_eq!(fs::read(&outputs[index]).unwrap(), expected);
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn native_batch_multi_function_objects_link_and_return() {
    let Some(linker) = find_command(&["cc", "gcc", "clang"]) else {
        eprintln!("C linker not found; skipping linked execution");
        return;
    };
    let directory = TestDirectory::new();
    let forward = directory.path.join("forward.kfr");
    let recursive = directory.path.join("recursive.kfr");
    fs::write(&forward, FORWARD_SOURCE).unwrap();
    fs::write(&recursive, RECURSIVE_SOURCE).unwrap();

    for (mode, prefix) in [
        (native_batch::BatchMode::Dag, "dag"),
        (native_batch::BatchMode::Pipeline, "pipeline"),
    ] {
        let output = directory.path.join(prefix);
        let objects =
            native_batch::compile_batch_with_mode([&forward, &recursive], &output, 2, mode)
                .unwrap();
        for (index, expected) in [42, 120].into_iter().enumerate() {
            let executable = output.join(format!("program-{index}"));
            let linked = Command::new(linker)
                .arg(&objects[index])
                .arg("-o")
                .arg(&executable)
                .output()
                .unwrap();
            assert!(
                linked.status.success(),
                "{linker} failed:\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&linked.stdout),
                String::from_utf8_lossy(&linked.stderr)
            );
            assert_eq!(
                Command::new(&executable).status().unwrap().code(),
                Some(expected)
            );
        }
    }
}

fn assert_object(object: &[u8], functions: &[&str], calls: &[&str]) {
    assert_eq!(&object[..4], b"\x7fELF");
    assert_eq!(read_u16(object, 16), 1, "object must be relocatable");
    assert_eq!(read_u16(object, 18), 62, "object must target x86_64");

    let symbols = section_contents(object, 3);
    let names: Vec<_> = (0..symbols.len() / 24)
        .filter_map(|index| {
            let symbol = &symbols[index * 24..(index + 1) * 24];
            (read_u16(symbol, 6) == 1 && symbol[4] & 0x0f == 2)
                .then(|| symbol_name(object, symbol).to_owned())
        })
        .collect();
    assert_eq!(names, functions);

    let relocations = section_contents(object, 2);
    assert_eq!(relocations.len(), calls.len() * 24);
    for (index, expected) in calls.iter().enumerate() {
        let relocation = &relocations[index * 24..(index + 1) * 24];
        let info = read_u64(relocation, 8);
        assert_eq!(info as u32, 4, "relocation must be R_X86_64_PLT32");
        assert_eq!(read_i64(relocation, 16), -4);
        let symbol = &symbols[(info >> 32) as usize * 24..((info >> 32) as usize + 1) * 24];
        assert_eq!(symbol_name(object, symbol), *expected);
    }
}

fn section_contents(object: &[u8], index: usize) -> &[u8] {
    let headers = read_u64(object, 40) as usize;
    let header = headers + index * 64;
    let offset = read_u64(object, header + 24) as usize;
    let size = read_u64(object, header + 32) as usize;
    &object[offset..offset + size]
}

fn symbol_name<'a>(object: &'a [u8], symbol: &[u8]) -> &'a str {
    let strings = section_contents(object, 4);
    let start = read_u32(symbol, 0) as usize;
    let length = strings[start..].iter().position(|byte| *byte == 0).unwrap();
    std::str::from_utf8(&strings[start..start + length]).unwrap()
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn find_command<'a>(commands: &'a [&'a str]) -> Option<&'a str> {
    commands.iter().copied().find(|command| {
        Command::new(command)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

#[test]
#[ignore = "multi-input throughput benchmark; run explicitly"]
fn pipelined_multi_input_throughput_comparison() {
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    const INPUTS: usize = 64;
    const TERMS: usize = 40;
    const ROUNDS: usize = 7;
    let jobs = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(4);
    let inputs: Vec<_> = (0..INPUTS)
        .map(|index| {
            let mut source = String::from("fn main(value: i64) -> i64 {\nlet v0 = value + 1;\n");
            for term in 1..TERMS {
                source.push_str(&format!(
                    "let v{term} = v{} * {} + {};\n",
                    term - 1,
                    term % 7 + 2,
                    index + term,
                ));
            }
            source.push_str(&format!("return v{};\n}}", TERMS - 1));
            (format!("input_{index:03}"), source)
        })
        .collect();

    fn measure(inputs: &[(String, String)], jobs: usize) -> (Duration, Vec<Vec<u8>>) {
        let start = Instant::now();
        let functions =
            pipelined_native::compile_sources_with_jobs(inputs.to_vec(), 8, jobs).unwrap();
        let elapsed = start.elapsed();
        let objects: Vec<_> = functions
            .iter()
            .map(|function| function.emit_object().unwrap())
            .collect();
        black_box(&objects);
        (elapsed, objects)
    }

    measure(&inputs, 1);
    measure(&inputs, jobs);
    let mut baseline_times = Vec::new();
    let mut candidate_times = Vec::new();
    for round in 0..ROUNDS {
        let (baseline, candidate) = if round % 2 == 0 {
            let baseline = measure(&inputs, 1);
            let candidate = measure(&inputs, jobs);
            (baseline, candidate)
        } else {
            let candidate = measure(&inputs, jobs);
            let baseline = measure(&inputs, 1);
            (baseline, candidate)
        };
        assert_eq!(baseline.1, candidate.1);
        println!(
            "round={} baseline_ms={:.3} jobs={jobs} candidate_ms={:.3}",
            round + 1,
            baseline.0.as_secs_f64() * 1000.0,
            candidate.0.as_secs_f64() * 1000.0,
        );
        baseline_times.push(baseline.0);
        candidate_times.push(candidate.0);
    }
    baseline_times.sort_unstable();
    candidate_times.sort_unstable();
    let baseline_median = baseline_times[ROUNDS / 2];
    let candidate_median = candidate_times[ROUNDS / 2];
    println!(
        "SUMMARY inputs={INPUTS} terms={TERMS} rounds={ROUNDS} jobs={jobs} baseline_median_ms={:.3} candidate_median_ms={:.3} median_speedup={:.3}x",
        baseline_median.as_secs_f64() * 1000.0,
        candidate_median.as_secs_f64() * 1000.0,
        baseline_median.as_secs_f64() / candidate_median.as_secs_f64(),
    );
}
