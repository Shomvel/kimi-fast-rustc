use kimi_fast_rustc::x86::{
    Condition, MachineFunction, MachineInstr, Operand, Reg, optimize_zero_idioms,
};
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn function(instructions: Vec<MachineInstr>) -> MachineFunction {
    let mut function = MachineFunction::new("zero_idiom");
    function.instructions = instructions;
    function
}

fn zero(register: Reg) -> MachineInstr {
    MachineInstr::MovImm {
        dst: register,
        value: 0,
    }
}

fn self_xor(register: Reg) -> MachineInstr {
    MachineInstr::Xor {
        dst: register,
        src: Operand::Reg(register),
    }
}

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

fn conditions() -> [(Condition, bool); 10] {
    [
        (Condition::Eq, false),
        (Condition::NotEq, true),
        (Condition::SignedLess, true),
        (Condition::SignedLessEq, true),
        (Condition::SignedGreater, false),
        (Condition::SignedGreaterEq, false),
        (Condition::UnsignedBelow, true),
        (Condition::UnsignedBelowEq, true),
        (Condition::UnsignedAbove, false),
        (Condition::UnsignedAboveEq, false),
    ]
}

#[test]
fn exact_bytes_save_four_bytes_per_zero_immediate() {
    let original = function(vec![
        zero(Reg::Rax),
        zero(Reg::R8),
        MachineInstr::MovImm {
            dst: Reg::Rcx,
            value: 1,
        },
        MachineInstr::Ret,
    ]);
    let baseline_size = original.emit_machine_code().unwrap().len();
    let mut optimized = original;
    assert_eq!(optimize_zero_idioms(&mut optimized.instructions), 2);
    assert_eq!(
        optimized.instructions,
        [
            self_xor(Reg::Rax),
            self_xor(Reg::R8),
            MachineInstr::MovImm {
                dst: Reg::Rcx,
                value: 1,
            },
            MachineInstr::Ret,
        ]
    );
    let code = optimized.emit_machine_code().unwrap();
    assert_eq!(
        code,
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0x31, 0xc0, 0x4d, 0x31, 0xc0, 0x48, 0xc7, 0xc1, 0x01,
            0x00, 0x00, 0x00, 0xc9, 0xc3,
        ]
    );
    assert_eq!(baseline_size - code.len(), 8);
    assert_eq!(machine_eval::evaluate(&optimized, &[]), Ok(0));
}

#[test]
fn cmp_to_setcc_preserves_flags_for_every_condition() {
    for (condition, expected) in conditions() {
        let mut function = function(vec![
            MachineInstr::MovImm {
                dst: Reg::Rax,
                value: 4,
            },
            MachineInstr::MovImm {
                dst: Reg::Rcx,
                value: 5,
            },
            MachineInstr::Cmp {
                lhs: Reg::Rax,
                rhs: Operand::Reg(Reg::Rcx),
            },
            zero(Reg::Rdx),
            MachineInstr::SetCc {
                condition,
                dst: Reg::Rax,
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(
            optimize_zero_idioms(&mut function.instructions),
            0,
            "{condition:?}"
        );
        assert_eq!(function.instructions[3], zero(Reg::Rdx));
        assert_eq!(
            machine_eval::evaluate(&function, &[]),
            Ok(i64::from(expected)),
            "{condition:?}"
        );
    }
}

#[test]
fn cmp_to_jcc_preserves_flags_for_every_condition() {
    for (condition, expected) in conditions() {
        let mut function = function(vec![
            MachineInstr::MovImm {
                dst: Reg::Rax,
                value: 4,
            },
            MachineInstr::MovImm {
                dst: Reg::Rcx,
                value: 5,
            },
            MachineInstr::Cmp {
                lhs: Reg::Rax,
                rhs: Operand::Reg(Reg::Rcx),
            },
            zero(Reg::Rdx),
            MachineInstr::Jcc {
                condition,
                target: 7,
            },
            zero(Reg::Rax),
            MachineInstr::Ret,
            MachineInstr::Label(7),
            MachineInstr::MovImm {
                dst: Reg::Rax,
                value: 1,
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(
            optimize_zero_idioms(&mut function.instructions),
            0,
            "{condition:?}"
        );
        assert_eq!(function.instructions[3], zero(Reg::Rdx));
        assert_eq!(
            machine_eval::evaluate(&function, &[]),
            Ok(i64::from(expected)),
            "{condition:?}"
        );
    }
}

#[test]
fn test_to_jcc_and_dead_flags_allow_the_correct_sites() {
    let mut protected = function(vec![
        MachineInstr::Test { reg: Reg::Rax },
        zero(Reg::Rdx),
        MachineInstr::Jcc {
            condition: Condition::Eq,
            target: 4,
        },
        MachineInstr::Ret,
        MachineInstr::Label(4),
        MachineInstr::Ret,
    ]);
    assert_eq!(optimize_zero_idioms(&mut protected.instructions), 0);
    assert_eq!(protected.instructions[1], zero(Reg::Rdx));

    let mut setcc = function(vec![
        MachineInstr::Test { reg: Reg::Rax },
        zero(Reg::Rdx),
        MachineInstr::SetCc {
            condition: Condition::Eq,
            dst: Reg::Rax,
        },
        MachineInstr::Ret,
    ]);
    assert_eq!(optimize_zero_idioms(&mut setcc.instructions), 0);
    assert_eq!(setcc.instructions[1], zero(Reg::Rdx));
    assert_eq!(machine_eval::evaluate(&setcc, &[]), Ok(1));

    let mut dead = function(vec![
        zero(Reg::Rdx),
        MachineInstr::Cmp {
            lhs: Reg::Rax,
            rhs: Operand::Reg(Reg::Rcx),
        },
        MachineInstr::SetCc {
            condition: Condition::Eq,
            dst: Reg::Rax,
        },
        zero(Reg::Rdx),
        MachineInstr::Ret,
    ]);
    assert_eq!(optimize_zero_idioms(&mut dead.instructions), 2);
    assert_eq!(dead.instructions[0], self_xor(Reg::Rdx));
    assert_eq!(dead.instructions[3], self_xor(Reg::Rdx));
    assert_eq!(machine_eval::evaluate(&dead, &[]), Ok(1));
}

#[test]
fn call_multiple_return_and_loop_label_boundaries_are_conservative() {
    let mut call = function(vec![
        zero(Reg::Rdx),
        MachineInstr::Call { target: 0 },
        MachineInstr::Ret,
    ]);
    call.symbols.push("target".into());
    assert_eq!(optimize_zero_idioms(&mut call.instructions), 0);
    assert_eq!(call.instructions[0], zero(Reg::Rdx));

    let mut returns = function(vec![
        zero(Reg::Rdx),
        MachineInstr::Ret,
        zero(Reg::Rcx),
        MachineInstr::Ret,
    ]);
    assert_eq!(optimize_zero_idioms(&mut returns.instructions), 0);
    assert_eq!(returns.instructions[0], zero(Reg::Rdx));
    assert_eq!(returns.instructions[2], zero(Reg::Rcx));

    let mut loop_header = function(vec![
        MachineInstr::Label(9),
        zero(Reg::Rdx),
        MachineInstr::Jmp(9),
        MachineInstr::Ret,
    ]);
    assert_eq!(optimize_zero_idioms(&mut loop_header.instructions), 0);
    assert_eq!(loop_header.instructions[1], zero(Reg::Rdx));

    let mut conditional_back_edge = function(vec![
        MachineInstr::Label(9),
        zero(Reg::Rdx),
        MachineInstr::Jcc {
            condition: Condition::NotEq,
            target: 9,
        },
        MachineInstr::Ret,
    ]);
    assert_eq!(
        optimize_zero_idioms(&mut conditional_back_edge.instructions),
        0
    );
    assert_eq!(conditional_back_edge.instructions[1], zero(Reg::Rdx));
}

#[test]
fn native_zero_return_uses_xor_and_saves_four_bytes() {
    let function = compile("fn main() -> i64 { return 0; }");
    assert_eq!(
        function.instructions,
        [
            self_xor(Reg::Rcx),
            MachineInstr::Mov {
                dst: Reg::Rax,
                src: Reg::Rcx,
            },
            MachineInstr::Ret,
        ]
    );
    assert_eq!(
        function.emit_machine_code().unwrap(),
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0x31, 0xc9, 0x48, 0x89, 0xc8, 0xc9, 0xc3,
        ]
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(0));
}

#[test]
fn native_branch_example_uses_zero_idioms_and_returns_42() {
    let function = compile(
        "fn main(value: i64) -> i64 { let zero = 0; return if value == 0 { 42 } else { zero }; }",
    );
    let replacements = function
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction,
                MachineInstr::Xor { dst, src: Operand::Reg(src) } if dst == src
            )
        })
        .count();
    assert_eq!(replacements, 1, "{:?}", function.instructions);
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(0));
}

#[test]
fn native_loop_preserves_conditions_and_returns_42() {
    let function = compile(
        "fn main() -> i64 { let mut total = 0; let mut i = 0; while i < 6 { total += 7; i += 1; } return total; }",
    );
    assert!(
        function
            .instructions
            .windows(2)
            .any(|instructions| matches!(
                instructions,
                [MachineInstr::Cmp { .. }, MachineInstr::Jcc { .. }]
            ))
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
    function.emit_machine_code().unwrap();
}
