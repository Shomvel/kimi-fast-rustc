use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn mutable_accumulator_runs_zero_or_many_iterations() {
    let function = compile(
        "fn main(limit: i64) -> i64 { let mut sum = 0; let mut i = 0; while i < limit { sum = sum + i; i = i + 1; } return sum; }",
    );
    let assembly = function.emit_gnu_assembly().unwrap();
    for (limit, expected) in [(0, 0), (1, 0), (2, 1), (10, 45), (100, 4950)] {
        assert_eq!(
            machine_eval::evaluate(&function, &[limit]),
            Ok(expected),
            "{assembly}"
        );
    }
    assert!(assembly.contains("\tje "));
    assert!(assembly.contains("\tjmp .L"));
}

#[test]
fn loop_phi_copies_preserve_sequential_assignments() {
    let function = compile(
        "fn main() -> i64 { let mut x = 1; let mut y = 2; let mut i = 0; while i < 1 { let next_x = y; let next_y = x; x = next_x; y = next_y; i = i + 1; } return x * 10 + y; }",
    );
    assert_eq!(
        machine_eval::evaluate(&function, &[]),
        Ok(21),
        "{}",
        function.emit_gnu_assembly().unwrap()
    );
}

#[test]
fn nested_loops_and_body_bindings_preserve_values() {
    let function = compile(
        "fn main(limit: i64) -> i64 { let mut total = 0; let mut i = 0; while i < limit { let mut j = 0; while j < i { total = total + j; j = j + 1; } i = i + 1; } return total; }",
    );
    let assembly = function.emit_gnu_assembly().unwrap();
    for (limit, expected) in [(0, 0), (1, 0), (2, 0), (4, 4), (8, 56)] {
        assert_eq!(
            machine_eval::evaluate(&function, &[limit]),
            Ok(expected),
            "{assembly}"
        );
    }
}

#[test]
fn loop_survives_register_pressure() {
    let function = compile(
        "fn main() -> i64 { let a = 1; let b = 2; let c = 3; let d = 4; let e = 5; let f = 6; let g = 7; let h = 8; let i = 9; let j = 10; let k = 11; let l = 12; let mut n = 0; let mut total = 0; while n < 10 { total = total + n; n = n + 1; } return total + a + b + c + d + e + f + g + h + i + j + k + l; }",
    );
    assert!(function.stack_slots > 0);
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(123));
}

#[test]
fn calls_and_relocations_survive_loops() {
    let graph = frontend::parse(
        "fn sum_to(limit: i64) -> i64 { let mut sum = 0; let mut i = 1; while i <= limit { sum = sum + i; i = i + 1; } return sum; } fn main() -> i64 { return sum_to(10); }",
    )
    .unwrap();
    let program = native_program::compile_graph(&graph).unwrap();
    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tcall sum_to\n"));
    assert!(assembly.contains("\tsetle "));
    assert!(assembly.contains("\tje "));
    let object = program.emit_object().unwrap();
    assert_eq!(&object[..4], b"\x7fELF");
    assert!(object.windows(10).any(|window| window == b".rela.text"));
}

#[test]
fn immutable_assignment_is_rejected() {
    let error = frontend::parse("fn main() { let x = 1; x = 2; }").unwrap_err();
    assert!(error.message.contains("immutable"));
}
