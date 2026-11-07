use kimi_fast_rustc::x86::{MachineFunction, MachineInstr, Operand};
use kimi_fast_rustc::{frontend, machine_eval, native, optimizer_pipeline, placement};

fn compile(source: &str) -> MachineFunction {
    let mut graph = frontend::parse(source).unwrap();
    optimizer_pipeline::optimize(&mut graph);
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

fn assert_no_constant_moves(function: &MachineFunction) {
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::MovImm { .. }))
    );
}

#[test]
fn left_constant_addition_uses_rhs_immediate() {
    let function = compile("fn main(value: i64) -> i64 { return 5 + value; }");
    assert_no_constant_moves(&function);
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::Add {
            src: Operand::Imm(5),
            ..
        }
    )));
    assert_eq!(machine_eval::evaluate(&function, &[37]), Ok(42));
}

#[test]
fn left_constant_multiplication_uses_rhs_immediate() {
    let function = compile("fn main(value: i64) -> i64 { return 3 * value; }");
    assert_no_constant_moves(&function);
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::IMul {
            src: Operand::Imm(3),
            ..
        }
    )));
    assert_eq!(machine_eval::evaluate(&function, &[5]), Ok(15));
}

#[test]
fn left_constant_bitwise_and_uses_rhs_immediate() {
    let function = compile("fn main(value: i64) -> i64 { return 7 & value; }");
    assert_no_constant_moves(&function);
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::And {
            src: Operand::Imm(7),
            ..
        }
    )));
    assert_eq!(machine_eval::evaluate(&function, &[5]), Ok(5));
}

#[test]
fn subtraction_keeps_the_constant_on_the_left() {
    let function = compile("fn main(value: i64) -> i64 { return 42 - value; }");
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::MovImm { value: 42, .. }))
    );
    assert_eq!(machine_eval::evaluate(&function, &[2]), Ok(40));
}
