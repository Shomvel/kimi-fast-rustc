use kimi_fast_rustc::x86::{Condition, MachineFunction, MachineInstr};
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

fn assert_no_boolean_materialization(function: &MachineFunction) {
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::SetCc { .. }))
    );
}

#[test]
fn if_condition_uses_flags_without_a_temporary_bool() {
    let function = compile("fn main(value: i64) -> i64 { return if value > 0 { 42 } else { 0 }; }");
    assert_no_boolean_materialization(&function);
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(0));
}

#[test]
fn loop_condition_uses_flags_without_a_temporary_bool() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; let mut i = 0; while i < 5 { sum += i; i += 1; } return sum + 32; }",
    );
    assert_no_boolean_materialization(&function);
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn actual_bool_results_keep_setcc_and_zero_extension() {
    let function = compile("fn main(value: i64) -> bool { return value == 0; }");
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::SetCc {
            condition: Condition::Eq,
            ..
        }
    )));
    let assembly = function.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tmovzx "));
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(1));
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(0));
}

#[test]
fn shared_comparison_still_materializes_its_bool_value() {
    let function = compile(
        "fn main(value: i64) -> i64 { let positive = value > 0; return if positive { positive as i64 } else { 0 }; }",
    );
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::SetCc {
            condition: Condition::SignedGreater,
            ..
        }
    )));
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(1));
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(0));
}
