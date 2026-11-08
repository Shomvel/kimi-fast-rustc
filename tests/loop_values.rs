use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

fn evaluate(source: &str, arguments: &[i64]) -> i64 {
    machine_eval::evaluate(&compile(source), arguments).unwrap()
}

#[test]
fn loop_expression_produces_the_break_value() {
    assert_eq!(
        evaluate(
            "fn main() -> i64 { let answer = loop { break 40 + 2; }; return answer; }",
            &[],
        ),
        42
    );
}

#[test]
fn loop_expression_can_be_the_function_tail() {
    assert_eq!(evaluate("fn main() -> i64 { loop { break 42; } }", &[]), 42);
}

#[test]
fn conditional_break_and_continue_preserve_exit_snapshots() {
    let function = compile(
        "fn main() -> i64 { let mut n = 0; let result = loop { n += 1; if n < 6 { continue; } break n; }; return result + n; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(12));
}

#[test]
fn nested_loop_expressions_target_the_innermost_loop() {
    let function = compile(
        "fn main() -> i64 { let mut outer = 0; let answer = loop { outer += 1; let inner = loop { break outer; }; if outer < 6 { continue; } break inner * 7; }; return answer; }",
    );
    assert_eq!(
        machine_eval::evaluate(&function, &[]),
        Ok(42),
        "{}",
        function.emit_gnu_assembly().unwrap()
    );
}

#[test]
fn nested_immediate_loops_preserve_result_types() {
    assert_eq!(
        evaluate(
            "fn main() -> u64 { let mut value: u64 = 40; let result = loop { let inner = loop { break value + 2; }; break inner; }; return result; }",
            &[],
        ),
        42
    );
}

#[test]
fn loop_result_respects_shadowing_and_restores_the_outer_binding() {
    assert_eq!(
        evaluate(
            "fn main() -> i64 { let value = 40; let answer = loop { let value = 41; if value == 41 { break value + 1; } break 0; }; return answer + value - 40; }",
            &[],
        ),
        42
    );
}

#[test]
fn break_literals_unify_with_other_breaks_and_the_expected_type() {
    let function = compile(
        "fn main(flag: bool) -> u64 { let result: u64 = loop { if flag { break 40; } break 2u64; }; return result; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(40));
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(2));
    assert_eq!(evaluate("fn main() -> u64 { loop { break 42; } }", &[]), 42);
}

#[test]
fn unit_loop_expressions_allow_break_without_a_value() {
    assert_eq!(
        evaluate(
            "fn main() { let result = loop { break; }; return result; }",
            &[],
        ),
        0
    );
    assert_eq!(evaluate("fn main() { loop { break; } }", &[]), 0);
    frontend::parse("fn main() { loop {} }")
        .unwrap()
        .validate()
        .unwrap();
}

#[test]
fn native_loop_value_example_returns_42() {
    assert_eq!(
        evaluate(include_str!("../examples/native_loop_value.kfr"), &[]),
        42
    );
}

#[test]
fn while_and_for_reject_break_values() {
    for source in [
        "fn main() { while true { break 42; } }",
        "fn main() { for i in 0..3 { break i; } }",
        "fn main() { for i in 0..=3 { break i; } }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error
                .message
                .contains("only supported in unconditional `loop` expressions"),
            "{error}; source: {source}"
        );
    }
}

#[test]
fn inconsistent_break_and_outer_types_are_rejected() {
    for source in [
        "fn main() -> i64 { let value = loop { if true { break 1i64; } break 2u64; }; return value; }",
        "fn main() -> i64 { let value = loop { if true { break 1; } break true; }; return value; }",
        "fn main() -> i64 { let value = loop { if true { break; } break 1; }; return value; }",
        "fn main() { let value: bool = loop { break 1; }; return value; }",
        "fn main() -> i64 { return loop { break; }; }",
        "fn main() -> i64 { loop { break 1; } return 42; }",
        "fn main() { loop { break 1; } }",
        "fn main() { return loop { break 1; }; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains("type mismatch"),
            "{error}; source: {source}"
        );
    }
}
