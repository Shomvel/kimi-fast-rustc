//! Machine allocation regressions migrated from the retired Graph backend.
use super::*;
use crate::machine::eval::{self, MachineEvalError};

type BinaryInstruction = fn(VirtualReg, VirtualValue, VirtualValue) -> VirtualInstr;
fn add(dst: VirtualReg, lhs: VirtualValue, rhs: VirtualValue) -> VirtualInstr {
    VirtualInstr::Add { dst, lhs, rhs }
}
fn sub(dst: VirtualReg, lhs: VirtualValue, rhs: VirtualValue) -> VirtualInstr {
    VirtualInstr::Sub { dst, lhs, rhs }
}
fn mul(dst: VirtualReg, lhs: VirtualValue, rhs: VirtualValue) -> VirtualInstr {
    VirtualInstr::Mul { dst, lhs, rhs }
}
fn div(dst: VirtualReg, lhs: VirtualValue, rhs: VirtualValue) -> VirtualInstr {
    VirtualInstr::Div { dst, lhs, rhs }
}
fn rem(dst: VirtualReg, lhs: VirtualValue, rhs: VirtualValue) -> VirtualInstr {
    VirtualInstr::Rem { dst, lhs, rhs }
}

#[derive(Default)]
struct Instructions {
    instructions: Vec<VirtualInstr>,
    next: u32,
}
impl Instructions {
    fn emit(&mut self, make: impl FnOnce(VirtualReg) -> VirtualInstr) -> VirtualValue {
        let dst = VirtualReg(self.next);
        self.next += 1;
        self.instructions.push(make(dst));
        VirtualValue::Reg(dst)
    }
    fn constant(&mut self, value: i64) -> VirtualValue {
        VirtualValue::Imm(value)
    }
    fn parameter(&mut self, index: usize) -> VirtualValue {
        self.emit(|dst| VirtualInstr::LoadArg { dst, index })
    }
    fn binary(
        &mut self,
        make: BinaryInstruction,
        lhs: VirtualValue,
        rhs: VirtualValue,
    ) -> VirtualValue {
        self.emit(|dst| make(dst, lhs, rhs))
    }
    fn compare(
        &mut self,
        condition: super::super::x86::Condition,
        lhs: VirtualValue,
        rhs: VirtualValue,
    ) -> VirtualValue {
        self.emit(|dst| VirtualInstr::Compare {
            dst,
            condition,
            lhs,
            rhs,
        })
    }
    fn ret(&mut self, value: VirtualValue) {
        self.instructions.push(VirtualInstr::Return(value));
    }
}

// These fixtures are straight-line SSA instruction lists. Production CFG
// liveness remains in analysis::liveness; no source graph enters this oracle.
fn compile(code: &Instructions, name: &str) -> Result<MachineFunction, CompileError> {
    let mut intervals: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for (position, instruction) in code.instructions.iter().enumerate() {
        let position = position as u32;
        if let Some(dst) = defined_register(instruction) {
            assert!(intervals.insert(dst.0, (position, position + 1)).is_none());
        }
        for src in used_registers(instruction) {
            intervals
                .get_mut(&src.0)
                .expect("SSA use must follow definition")
                .1 = position + 1;
        }
    }
    compile_virtual_function(
        code.instructions.clone(),
        intervals
            .into_iter()
            .map(|(reg, (start, end))| LiveInterval::new(AllocVirtualReg(reg), start, end))
            .collect(),
        name,
    )
}
fn run(function: &MachineFunction, args: &[i64]) -> i64 {
    eval::evaluate(function, args).unwrap()
}
fn sequence(start: i64, count: usize) -> Vec<i64> {
    (start..start + count as i64).collect()
}
use super::super::x86::Condition;

#[test]
fn compiles_constants_arithmetic_and_gnu_assembly() {
    let mut code = Instructions::default();
    let seven = code.constant(7);
    let three = code.constant(3);
    let difference = code.binary(sub, seven, three);
    let two = code.constant(2);
    let product = code.binary(mul, difference, two);
    let five = code.constant(5);
    let sum = code.binary(add, product, five);
    code.ret(sum);

    let function = compile(&code, "arithmetic").unwrap();
    assert_eq!(run(&function, &[]), 13);
    assert_eq!(function.stack_slots, 0);
    assert!(function.callee_saved.is_empty());

    let assembly = compile(&code, "arithmetic")
        .unwrap()
        .emit_gnu_assembly()
        .unwrap();
    assert!(assembly.starts_with(".intel_syntax noprefix\n.text\n.globl arithmetic\n"));
    assert!(assembly.contains("\timul"));
    assert!(
        assembly
            .ends_with(".size arithmetic, .-arithmetic\n.section .note.GNU-stack,\"\",@progbits\n")
    );
}

#[test]
fn signed_division_and_remainder_use_rax_rdx_in_lowering_order() {
    assert!(!GENERAL_REGISTERS.contains(&Reg::Rax));
    assert!(!GENERAL_REGISTERS.contains(&Reg::Rdx));

    for (operation, lhs, rhs, expected, result_register) in [
        (div as BinaryInstruction, -42, 4, -10, Reg::Rax),
        (rem as BinaryInstruction, -42, 4, -2, Reg::Rdx),
        (div as BinaryInstruction, 42, -4, -10, Reg::Rax),
        (rem as BinaryInstruction, 42, -4, 2, Reg::Rdx),
        (div as BinaryInstruction, i64::MIN, 1, i64::MIN, Reg::Rax),
        (rem as BinaryInstruction, i64::MIN, 1, 0, Reg::Rdx),
    ] {
        let mut code = Instructions::default();
        let lhs_node = code.parameter(0);
        let rhs_node = code.parameter(1);
        let result = code.binary(operation, lhs_node, rhs_node);
        code.ret(result);

        let function = compile(&code, "signed_division").unwrap();
        assert_eq!(run(&function, &[lhs, rhs]), expected);
        assert_eq!(eval::evaluate(&function, &[lhs, rhs]), Ok(expected));
        let idiv = function
            .instructions
            .iter()
            .position(|instruction| matches!(instruction, MachineInstr::IDiv { .. }))
            .unwrap();
        assert_eq!(function.instructions[idiv - 1], MachineInstr::Cqo);
        match function.instructions[idiv] {
            MachineInstr::IDiv {
                src: Operand::Reg(divisor),
            } => assert!(!matches!(divisor, Reg::Rax | Reg::Rdx)),
            MachineInstr::IDiv {
                src: Operand::Stack(_),
            } => {}
            _ => unreachable!(),
        }
        assert!(matches!(
            function.instructions[idiv + 1],
            MachineInstr::Mov { src, .. } if src == result_register
        ));

        let assembly = function.emit_gnu_assembly().unwrap();
        assert!(assembly.contains("\tcqo\n\tidiv "), "{assembly}");
    }
}

#[test]
fn division_preserves_later_rdx_arguments_and_reads_stack_arguments() {
    for (operation, expected) in [
        (div as BinaryInstruction, 11),
        (rem as BinaryInstruction, 18),
    ] {
        let mut code = Instructions::default();
        let lhs = code.parameter(0);
        let rhs = code.parameter(1);
        let division = code.binary(operation, lhs, rhs);
        let third = code.parameter(2);
        let seventh = code.parameter(6);
        let product = code.binary(mul, third, seventh);
        let result = code.binary(add, division, product);
        code.ret(result);

        let arguments = [-43, 4, 7, 0, 0, 0, 3];
        let function = compile(&code, "division_arguments").unwrap();
        assert!(matches!(
            function.instructions.first(),
            Some(MachineInstr::Store { src: Reg::Rdx, .. })
        ));
        assert_eq!(run(&function, &arguments), expected);
        assert_eq!(eval::evaluate(&function, &arguments), Ok(expected));
    }
}

#[test]
fn division_and_remainder_accept_spilled_divisors() {
    for (operation, expected) in [
        (div as BinaryInstruction, 207),
        (rem as BinaryInstruction, 208),
    ] {
        let mut code = Instructions::default();
        let mut values = Vec::new();
        for value in 1..=20 {
            values.push(code.parameter((value - 1) as usize));
        }
        let mut sum = values[2];
        for value in values.iter().skip(3) {
            sum = code.binary(add, sum, *value);
        }
        let division = code.binary(operation, values[0], values[1]);
        let result = code.binary(add, sum, division);
        code.ret(result);

        let function = compile(&code, "spilled_divisor").unwrap();
        assert!(function.stack_slots > 0);
        assert!(function.instructions.iter().any(|instruction| matches!(
            instruction,
            MachineInstr::IDiv {
                src: Operand::Stack(_)
            }
        )));
        assert_eq!(run(&function, &sequence(1, 20)), expected);
        assert_eq!(eval::evaluate(&function, &sequence(1, 20)), Ok(expected));
    }
}

#[test]
fn division_and_remainder_spill_results_without_losing_them() {
    for (operation, expected) in [
        (div as BinaryInstruction, 200),
        (rem as BinaryInstruction, 207),
    ] {
        let mut code = Instructions::default();
        let lhs = code.constant(-43);
        let rhs = code.constant(4);
        let division = code.binary(operation, lhs, rhs);
        let mut values = Vec::new();
        for value in 1..=20 {
            values.push(code.parameter((value - 1) as usize));
        }
        let mut sum = values[0];
        for value in values.into_iter().skip(1) {
            sum = code.binary(add, sum, value);
        }
        let result = code.binary(add, sum, division);
        code.ret(result);

        let function = compile(&code, "spilled_division_result").unwrap();
        let result_position = if expected == 200 {
            function
                .instructions
                .iter()
                .position(|instruction| {
                    matches!(instruction, MachineInstr::ShrImm { count: 2, .. })
                })
                .unwrap()
                + 1
        } else {
            function
                .instructions
                .iter()
                .position(|instruction| matches!(instruction, MachineInstr::IDiv { .. }))
                .unwrap()
                + 2
        };
        assert!(matches!(
            function.instructions[result_position],
            MachineInstr::Store { .. }
        ));
        assert_eq!(run(&function, &sequence(1, 20)), expected);
        assert_eq!(eval::evaluate(&function, &sequence(1, 20)), Ok(expected));
    }
}

#[test]
fn unused_trapping_division_and_remainder_are_not_discarded() {
    for (operation, lhs, rhs, expected) in [
        (
            div as BinaryInstruction,
            1,
            0,
            MachineEvalError::DivisionByZero,
        ),
        (
            rem as BinaryInstruction,
            1,
            0,
            MachineEvalError::DivisionByZero,
        ),
        (
            div as BinaryInstruction,
            i64::MIN,
            -1,
            MachineEvalError::DivisionOverflow,
        ),
        (
            rem as BinaryInstruction,
            i64::MIN,
            -1,
            MachineEvalError::DivisionOverflow,
        ),
    ] {
        let mut code = Instructions::default();
        let lhs = code.constant(lhs);
        let rhs = code.constant(rhs);
        code.binary(operation, lhs, rhs);
        let one = code.constant(1);
        code.ret(one);

        let function = compile(&code, "trapping_division").unwrap();
        assert!(
            function
                .instructions
                .iter()
                .any(|instruction| matches!(instruction, MachineInstr::IDiv { .. }))
        );
        assert_eq!(eval::evaluate(&function, &[]), Err(expected));
    }
}

#[test]
fn supports_register_stack_and_repeated_parameters() {
    let mut code = Instructions::default();
    let first = code.parameter(0);
    let second = code.parameter(1);
    let seventh = code.parameter(6);
    let sum = code.binary(add, first, second);
    let product = code.binary(mul, sum, seventh);
    let repeated = code.parameter(0);
    let result = code.binary(add, product, repeated);
    code.ret(result);

    let args = [9, 4, 0, 0, 0, 0, 3];
    assert_eq!(run(&compile(&code, "params").unwrap(), &args), 48);

    let assembly = compile(&code, "params")
        .unwrap()
        .emit_gnu_assembly()
        .unwrap();
    assert!(assembly.contains("QWORD PTR [rbp+16]"), "{assembly}");
}

#[test]
fn parameter_homes_preserve_clobbered_incoming_registers() {
    let mut code = Instructions::default();
    let first = code.parameter(0);
    let mut values = Vec::new();
    for value in 0..12 {
        values.push(code.parameter(value as usize));
    }
    let second = code.parameter(1);
    let mut sum = code.binary(add, first, second);
    for value in values {
        sum = code.binary(add, sum, value);
    }
    code.ret(sum);

    let function = compile(&code, "argument_home").unwrap();
    assert!(function.stack_slots >= 1);
    let mut arguments = sequence(0, 12);
    arguments[0] = 20;
    arguments[1] = 7;
    assert_eq!(run(&function, &arguments), 119);
    assert!(matches!(
        function.instructions.first(),
        Some(MachineInstr::Store { src: Reg::Rsi, .. })
    ));
}

#[test]
fn high_pressure_spills_definitions_and_uses() {
    let mut code = Instructions::default();
    let mut values = Vec::new();
    for value in 1..=20 {
        values.push(code.parameter((value - 1) as usize));
    }
    let mut sum = values[0];
    for value in values.into_iter().skip(1) {
        sum = code.binary(add, sum, value);
    }
    code.ret(sum);

    let function = compile(&code, "pressure").unwrap();
    assert!(function.stack_slots > 0);
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::Store { .. }))
    );
    assert!(function.instructions.iter().any(|instruction| {
        matches!(instruction, MachineInstr::Load { .. })
            || matches!(
                instruction,
                MachineInstr::Add {
                    src: Operand::Stack(_),
                    ..
                }
            )
    }));
    assert_eq!(run(&function, &sequence(1, 20)), 210);
    assert!(
        function
            .callee_saved
            .iter()
            .all(|register| register.is_callee_saved())
    );
}

#[test]
fn every_signed_comparison_returns_zero_or_one() {
    let cases = [
        (Condition::Eq, [0, 1, 0]),
        (Condition::NotEq, [1, 0, 1]),
        (Condition::SignedLess, [1, 0, 0]),
        (Condition::SignedLessEq, [1, 1, 0]),
        (Condition::SignedGreater, [0, 0, 1]),
        (Condition::SignedGreaterEq, [0, 1, 1]),
    ];

    for (operation, expected) in cases {
        let mut code = Instructions::default();
        let lhs = code.parameter(0);
        let rhs = code.parameter(1);
        let comparison = code.compare(operation, lhs, rhs);
        code.ret(comparison);
        let function = compile(&code, "compare").unwrap();
        for ((lhs, rhs), expected) in [(4, 5), (5, 5), (6, 5)].into_iter().zip(expected) {
            assert_eq!(run(&function, &[lhs, rhs]), expected, "{operation:?}");
        }
    }
}

#[test]
fn spilled_compare_uses_scratch_and_stores_boolean() {
    let mut code = Instructions::default();
    let mut values = Vec::new();
    for value in 0..18 {
        values.push(code.parameter(value as usize));
    }
    let comparison = code.compare(Condition::SignedLess, values[0], values[1]);
    let mut sum = values[2];
    for value in values.into_iter().skip(3) {
        sum = code.binary(add, sum, value);
    }
    let result = code.binary(add, sum, comparison);
    code.ret(result);

    let function = compile(&code, "spilled_compare").unwrap();
    assert!(
        function.instructions.iter().any(|instruction| {
            matches!(instruction, MachineInstr::SetCc { dst: SCRATCH0, .. })
        })
    );
    assert_eq!(run(&function, &sequence(0, 18)), 153);
}

#[test]
fn subtraction_keeps_lhs_and_rhs_order_even_with_spills() {
    let mut code = Instructions::default();
    let mut values = Vec::new();
    for value in 0..16 {
        values.push(code.parameter(value as usize));
    }
    let difference = code.binary(sub, values[0], values[15]);
    let mut sum = difference;
    for value in values.into_iter().skip(1).take(14) {
        sum = code.binary(add, sum, value);
    }
    code.ret(sum);

    let function = compile(&code, "sub_spills").unwrap();
    assert!(function.stack_slots > 0);
    assert_eq!(run(&function, &sequence(0, 16)), 90);
}

#[test]
fn supports_multiple_returns() {
    let mut code = Instructions::default();
    let one = code.constant(1);
    code.ret(one);
    code.ret(one);
    let function = compile(&code, "returns").unwrap();
    assert_eq!(run(&function, &[]), 1);
    assert_eq!(
        function
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction, MachineInstr::Ret))
            .count(),
        2
    );
}

#[test]
fn missing_virtual_register_allocation_is_rejected() {
    let error = compile_virtual_function(
        vec![VirtualInstr::Return(VirtualValue::Reg(VirtualReg(7)))],
        vec![],
        "missing_allocation",
    )
    .unwrap_err();
    assert_eq!(error, CompileError::MissingAllocation(VirtualReg(7)));
}

#[test]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn linked_call_preserves_live_values_and_stages_register_and_stack_arguments() {
    use crate::{FunctionCode, SymbolBinding, write_object};
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut code = Instructions::default();
    let arguments = (0..8)
        .map(|index| code.parameter(index))
        .collect::<Vec<_>>();
    let result = code.emit(|dst| VirtualInstr::Call {
        dst,
        target: "clobber".into(),
        args: arguments.clone(),
    });
    let mut sum = result;
    for argument in arguments {
        sum = code.binary(add, sum, argument);
    }
    code.ret(sum);
    let function = compile(&code, "preserve_call").unwrap();
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::StoreArg { index: 7, .. }))
    );
    let object = write_object(&[FunctionCode {
        function,
        binding: SymbolBinding::Global { hidden: false },
    }])
    .unwrap();
    let directory = std::env::temp_dir().join(format!(
        "kfrc-call-save-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("caller.o"), object).unwrap();
    // Consume all eight inputs, then destroy every allocatable caller-saved
    // register. The caller must recover its pre-call values from its own homes.
    fs::write(
        directory.join("clobber.S"),
        r#"
.text
.globl clobber
.type clobber, @function
clobber:
    mov %rdi, %rax
    add %rsi, %rax
    add %rdx, %rax
    add %rcx, %rax
    add %r8, %rax
    add %r9, %rax
    add 8(%rsp), %rax
    add 16(%rsp), %rax
    xor %rcx, %rcx
    xor %rdx, %rdx
    xor %rsi, %rsi
    xor %rdi, %rdi
    xor %r8, %r8
    xor %r9, %r9
    xor %r10, %r10
    xor %r11, %r11
    ret
.size clobber, .-clobber
.section .note.GNU-stack,"",@progbits
"#,
    )
    .unwrap();
    fs::write(
        directory.join("main.c"),
        r#"
#include <stdint.h>
extern int64_t preserve_call(int64_t,int64_t,int64_t,int64_t,int64_t,int64_t,int64_t,int64_t);
int main(void) {
    if (preserve_call(1,2,3,4,5,6,7,8) != 72) return 1;
    if (preserve_call(-10,2,30,-4,50,6,-70,8) != 24) return 2;
    return 0;
}
"#,
    )
    .unwrap();
    let linked = Command::new("cc")
        .current_dir(&directory)
        .args(["caller.o", "clobber.S", "main.c", "-o", "run"])
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let executed = Command::new(directory.join("run")).output().unwrap();
    fs::remove_dir_all(&directory).unwrap();
    assert!(executed.status.success(), "{executed:?}");
}
