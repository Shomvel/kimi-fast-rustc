use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn break_preserves_values_at_the_exit_edge() {
    let function = compile(
        "fn main() -> i64 { let mut value = 0; while true { value += 1; if value > 20 { break; } value += 10; } return value; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(23));
}

#[test]
fn continue_skips_only_the_rest_of_the_current_iteration() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; let mut i = 0; while i < 10 { i += 1; if i % 2 == 1 { continue; } sum += i; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(30));
}

#[test]
fn one_terminal_if_arm_still_merges_the_reachable_arm() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; let mut i = 0; while i < 6 { i += 1; if i % 2 == 0 { continue; } sum += i; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(9));
}

#[test]
fn continue_and_break_can_terminate_different_if_arms() {
    let function = compile(
        "fn main() -> i64 { let mut i = 0; while true { i += 1; if i < 3 { continue; } else { break; } } return i; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(3));
}

#[test]
fn nested_loop_control_targets_the_innermost_loop() {
    let function = compile(
        "fn main() -> i64 { let mut total = 0; let mut outer = 0; while outer < 3 { let mut inner = 0; while true { inner += 1; if inner == 2 { break; } total += 1; } outer += 1; } return total; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(3));
}

#[test]
fn break_exit_phi_preserves_bool_and_integer_types() {
    let function = compile(
        "fn main() -> i64 { let mut i = 0; let mut done = false; while !done { i += 1; if i == 3 { done = true; break; } } return if done && i == 3 { 42 } else { 0 }; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn invalid_loop_control_is_rejected_explicitly() {
    for (source, expected) in [
        ("fn main() { break; }", "outside a loop"),
        ("fn main() { continue; }", "outside a loop"),
        (
            "fn main() { while true { break; let value = 1; } }",
            "after `break`",
        ),
        (
            "fn main() { while true { continue; let value = 1; } }",
            "after `break`",
        ),
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains(expected),
            "{error}; source: {source}"
        );
    }
}

#[test]
fn evaluator_stops_an_actual_infinite_loop() {
    let function = compile("fn main() -> i64 { while true { } return 0; }");
    assert_eq!(
        machine_eval::evaluate(&function, &[]),
        Err(machine_eval::MachineEvalError::InstructionLimitExceeded)
    );
}
