use kimi_fast_rustc::parallel_native::ParallelCompileError;
use kimi_fast_rustc::scheduler::SchedulerError;
use kimi_fast_rustc::{frontend, native_program, parallel_native, placement};

const WORKLOAD: &str = "
    fn main() -> i64 { return later(40) + factorial(5); }
    fn later(value: i64) -> i64 { return value + 2; }
    fn factorial(value: i64) -> i64 {
        if value <= 1 { return 1; }
        return value * factorial(value - 1);
    }
    fn even(value: u64) -> bool {
        if value == 0 { return true; }
        return odd(value - 1);
    }
    fn odd(value: u64) -> bool {
        if value == 0 { return false; }
        return even(value - 1);
    }
";

#[test]
fn function_parallel_preserves_order_and_object_bytes() {
    let graph = frontend::parse(WORKLOAD).unwrap();
    let expected = native_program::compile_graph(&graph).unwrap();
    let expected_object = expected.emit_object().unwrap();
    let expected_names: Vec<_> = graph
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();

    for jobs in [1, 2, 3, 8] {
        let program = parallel_native::compile_graph(&graph, jobs).unwrap();
        let names: Vec<_> = program
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect();
        assert_eq!(names, expected_names, "jobs: {jobs}");
        assert_eq!(program.functions, expected.functions, "jobs: {jobs}");
        assert_eq!(
            program.emit_object().unwrap(),
            expected_object,
            "jobs: {jobs}"
        );
    }
}

#[test]
fn forward_and_recursive_calls_are_emitted_unchanged() {
    let graph = frontend::parse(WORKLOAD).unwrap();
    let sequential = native_program::compile_graph_to_assembly(&graph).unwrap();
    let parallel = parallel_native::compile_graph(&graph, 4)
        .unwrap()
        .emit_gnu_assembly()
        .unwrap();

    assert_eq!(parallel, sequential);
    for target in ["later", "factorial", "odd", "even"] {
        assert!(
            parallel.contains(&format!("\tcall {target}\n")),
            "missing call to {target}:\n{parallel}"
        );
    }
}

#[test]
fn function_parallel_rejects_zero_jobs() {
    let graph = frontend::parse(WORKLOAD).unwrap();
    assert_eq!(
        parallel_native::compile_graph(&graph, 0).unwrap_err(),
        ParallelCompileError::Scheduler(SchedulerError::ZeroParallelism)
    );
}

#[test]
fn optimized_strength_constants_survive_function_splitting() {
    let mut graph = frontend::parse(
        "fn main() -> i64 { return later(40) + twice(1); } fn later(value: i64) -> i64 { return value + 2; } fn twice(value: i64) -> i64 { return value * 2; }",
    )
    .unwrap();
    kimi_fast_rustc::optimizer_pipeline::optimize(&mut graph);
    let graph = placement::place_for_native(&graph).unwrap();
    let sequential = native_program::compile_graph(&graph).unwrap();
    let parallel = parallel_native::compile_graph(&graph, 4).unwrap();
    assert_eq!(parallel.functions, sequential.functions);
    assert_eq!(
        parallel.emit_object().unwrap(),
        sequential.emit_object().unwrap()
    );
}
