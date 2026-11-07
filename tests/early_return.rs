use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn early_if_return_skips_the_function_tail() {
    let function = compile("fn main(value: i64) -> i64 { if value > 10 { return 42; } return 7; }");
    for (value, expected) in [(11, 42), (100, 42), (10, 7), (0, 7), (-3, 7)] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
    assert_eq!(
        function
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction, kimi_fast_rustc::x86::MachineInstr::Ret))
            .count(),
        2
    );
}

#[test]
fn two_returning_arms_do_not_need_a_final_return() {
    let function =
        compile("fn main(value: i64) -> i64 { if value > 0 { return 42; } else { return 7; } }");
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(7));
}

#[test]
fn return_inside_loop_leaves_without_updating_loop_phis() {
    let function = compile(
        "fn main() -> i64 { let mut i = 0; while true { i += 1; if i == 4 { return i * 10 + 2; } } return 0; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn unit_functions_support_early_and_implicit_returns() {
    let early = compile("fn main() { if true { return; } }");
    assert_eq!(machine_eval::evaluate(&early, &[]), Ok(0));

    let both = compile("fn main(flag: bool) { if flag { return; } else { return; } }");
    assert_eq!(machine_eval::evaluate(&both, &[0]), Ok(0));
    assert_eq!(machine_eval::evaluate(&both, &[1]), Ok(0));
}

#[test]
fn bool_early_returns_work_across_function_calls() {
    let graph = frontend::parse(
        "fn decide(value: i64) -> bool { if value > 0 { return true; } return false; } fn main() -> i64 { return if decide(42) { 42 } else { 0 }; }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall decide\n"));
    assert!(assembly.matches("\tret\n").count() >= 3);
}

#[test]
fn early_return_preserves_type_and_reachability_errors() {
    let error = frontend::parse("fn main() -> i64 { if true { return; } return 1; }").unwrap_err();
    assert!(error.message.contains("expected a `i64` expression"));

    let error =
        frontend::parse("fn main(value: i64) -> i64 { if value > 0 { return 1; } }").unwrap_err();
    assert!(error.message.contains("must end with `return"));

    let error = frontend::parse("fn main() -> i64 { if true { return 1; let x = 2; } return 0; }")
        .unwrap_err();
    assert!(
        error
            .message
            .contains("after `break`, `continue`, or `return`")
    );
}
