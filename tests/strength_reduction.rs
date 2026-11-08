use kimi_fast_rustc::x86::{MachineFunction, MachineInstr};
use kimi_fast_rustc::{frontend, machine_eval, native, optimizer_pipeline, placement};

fn compile(source: &str) -> MachineFunction {
    let mut graph = frontend::parse(source).unwrap();
    optimizer_pipeline::optimize(&mut graph);
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn multiplication_by_power_of_two_becomes_left_shift() {
    let function = compile("fn main(value: i64) -> i64 { return value * 8; }");
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::Shl { .. } | MachineInstr::ShlImm { .. }
    )));
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::IMul { .. }))
    );
    assert_eq!(machine_eval::evaluate(&function, &[-2]), Ok(-16));
    assert_eq!(machine_eval::evaluate(&function, &[3]), Ok(24));
}

#[test]
fn power_of_two_on_the_left_also_becomes_left_shift() {
    let function = compile("fn main(value: i64) -> i64 { return 8 * value; }");
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::Shl { .. } | MachineInstr::ShlImm { .. }
    )));
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::IMul { .. }))
    );
    assert_eq!(machine_eval::evaluate(&function, &[5]), Ok(40));
}

#[test]
fn unsigned_division_by_power_of_two_becomes_logical_shift() {
    let function = compile("fn main(value: u64) -> u64 { return value / 8; }");
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::LShr { .. } | MachineInstr::LShrImm { .. }
    )));
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::UDiv { .. }))
    );
    assert_eq!(
        machine_eval::evaluate(&function, &[-1]),
        Ok(2305843009213693951i64)
    );
}

#[test]
fn signed_power_division_uses_shifts_and_non_power_multiplication_is_unchanged() {
    let division = compile("fn main(value: i64) -> i64 { return value / 8; }");
    assert!(division.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::Shr { .. } | MachineInstr::ShrImm { .. }
    )));
    assert!(
        !division
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::IDiv { .. }))
    );
    assert_eq!(machine_eval::evaluate(&division, &[-9]), Ok(-1));

    let multiplication = compile("fn main(value: i64) -> i64 { return value * 3; }");
    assert!(
        multiplication
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::IMul { .. }))
    );
    assert_eq!(machine_eval::evaluate(&multiplication, &[5]), Ok(15));
}
