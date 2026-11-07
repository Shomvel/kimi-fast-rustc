use kimi_fast_rustc::x86::{MachineFunction, MachineInstr};
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

fn assert_no_self_moves(function: &MachineFunction) {
    for instruction in &function.instructions {
        if let MachineInstr::Mov { dst, src } = instruction {
            assert_ne!(dst, src, "self move in {}", function.name);
        }
    }
}

#[test]
fn if_phi_moves_do_not_emit_self_assignments() {
    let function =
        compile("fn main(value: i64) -> i64 { return if value > 0 { value + 1 } else { value }; }");
    assert_no_self_moves(&function);
    assert_eq!(machine_eval::evaluate(&function, &[41]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(0));
}

#[test]
fn loop_back_edge_moves_do_not_emit_self_assignments() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; for i in 0..5 { sum += i; } return sum + 32; }",
    );
    assert_no_self_moves(&function);
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn calls_keep_only_required_argument_and_result_moves() {
    let graph = frontend::parse(
        "fn id(value: i64) -> i64 { return value; } fn main() -> i64 { return id(42); }",
    )
    .unwrap();
    let program = native_program::compile_graph(&graph).unwrap();
    for function in &program.functions {
        assert_no_self_moves(function);
    }
    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tcall id\n"));
    assert!(assembly.contains("\tmov rax,"));
}
