use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn if_else_merges_assignments_from_both_branches() {
    let function = compile(
        "fn main(value: i64) -> i64 { let mut result = 0; if value > 0 { let adjustment = 40; result = value + adjustment; } else { let adjustment = 7; result = adjustment; } return result; }",
    );
    for (value, expected) in [(2, 42), (1, 41), (0, 7), (-9, 7)] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn if_without_else_preserves_the_unmodified_value() {
    let function = compile(
        "fn main(value: i64) -> i64 { let mut result = 5; if value > 0 { result = result + value; } return result; }",
    );
    for (value, expected) in [(3, 8), (1, 6), (0, 5), (-4, 5)] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn nested_if_statements_merge_each_control_level() {
    let function = compile(
        "fn main(value: i64) -> i64 { let mut result = 0; if value > 0 { if value > 10 { result = 2; } else { result = 1; } } else { result = -1; } return result; }",
    );
    for (value, expected) in [(20, 2), (11, 2), (10, 1), (5, 1), (0, -1), (-3, -1)] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn if_inside_loop_accumulates_selected_values() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; let mut i = 0; while i < 10 { if i % 2 == 0 { sum = sum + i; } i = i + 1; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(20));
}

#[test]
fn branch_with_inner_loop_jumps_to_the_outer_merge() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; let mut i = 0; while i < 5 { if i < 3 { let mut j = 0; while j < 2 { sum = sum + 1; j = j + 1; } } else { sum = sum + 10; } i = i + 1; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(26));
}

#[test]
fn else_if_chains_merge_every_nested_branch() {
    let function = compile(
        "fn main(value: i64) -> i64 { let mut result = 0; if value > 20 { result = 3; } else if value > 10 { result = 2; } else if value > 0 { result = 1; } else { result = -1; } return result; }",
    );
    for (value, expected) in [
        (21, 3),
        (20, 2),
        (11, 2),
        (10, 1),
        (1, 1),
        (0, -1),
        (-8, -1),
    ] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn branch_locals_do_not_escape_and_terminal_statements_end_bodies() {
    let error =
        frontend::parse("fn main() { if true { let branch_local = 42; } return branch_local; }")
            .unwrap_err();
    assert!(error.message.contains("unknown variable"));

    let error = frontend::parse("fn main() { if true { return; let value = 1; } }").unwrap_err();
    assert!(
        error
            .message
            .contains("after `break`, `continue`, or `return`")
    );
}
