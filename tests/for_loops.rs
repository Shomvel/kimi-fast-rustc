use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn exclusive_ranges_accumulate_and_empty_ranges_do_not_run() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; for i in 0..5 { sum += i; } for i in 5..0 { sum += 100; } for i in 2..5 { sum += i; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(19));
}

#[test]
fn continue_advances_the_range_index() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; for i in 0..5 { if i % 2 == 1 { continue; } sum += i; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(6));
}

#[test]
fn break_exits_the_range_loop() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; for i in 0..10 { if i == 4 { break; } sum += i; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(6));
}

#[test]
fn nested_ranges_target_the_inner_loop_control_context() {
    let function = compile(
        "fn main() -> i64 { let mut total = 0; for outer in 0..4 { for inner in 0..4 { if inner == 2 { continue; } total += 1; } } return total; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(12));
}

#[test]
fn usize_is_a_u64_alias_in_ranges_and_bindings() {
    let function = compile(
        "fn main() -> usize { let mut total = 0usize; for i in 0..5usize { total += i; } return total; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(10));
}

#[test]
fn early_return_works_inside_a_range_loop() {
    let function =
        compile("fn main() -> i64 { for i in 0..10 { if i == 3 { return i + 39; } } return 0; }");
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn range_calls_and_invalid_syntax_are_checked() {
    let graph = frontend::parse(
        "fn start() -> usize { return 1; } fn end() -> usize { return 5; } fn main() -> usize { let mut sum = 0usize; for i in start()..end() { sum += i; } return sum; }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall start\n"));
    assert!(assembly.contains("\tcall end\n"));

    for (source, expected) in [
        ("fn main() { for i in 0..=3 { } }", "inclusive ranges"),
        ("fn main() { for i in 0..3 { i += 1; } }", "immutable"),
        (
            "fn main() { for i in 0..3 { } return i; }",
            "unknown variable",
        ),
        (
            "fn main(a: i64, b: u64) { for i in a..b { } }",
            "type mismatch",
        ),
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains(expected),
            "{error}; source: {source}"
        );
    }
}
