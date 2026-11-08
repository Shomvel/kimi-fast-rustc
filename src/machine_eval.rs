//! Reference evaluation for the machine functions emitted by `x86`.
//!
//! This models the instruction subset used by the native backend for differential
//! testing. It does not execute machine code and is not a complete x86 or System V
//! ABI simulator.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::x86::{Condition, MachineFunction, MachineInstr, Operand, Reg, StackSlot};

const REGISTER_COUNT: usize = 16;
const CALLEE_SAVED_INITIAL: [i64; Reg::CALLEE_SAVED.len()] = [
    0x1020_3040_5060_7001,
    0x1020_3040_5060_7002,
    0x1020_3040_5060_7003,
    0x1020_3040_5060_7004,
    0x1020_3040_5060_7005,
    0x1020_3040_5060_7006,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MachineEvalError {
    MissingReturn,
    MissingArgument { index: usize, available: usize },
    StackSlotOutOfBounds { slot: StackSlot, stack_slots: u32 },
    SetCcWithoutCmp,
    DivisionByZero,
    DivisionOverflow,
    UnknownLabel(u32),
    InstructionLimitExceeded,
    UnsupportedCall(u32),
    InvalidDivisor(Reg),
    ImmediateDivision,
    ReservedRegister(Reg),
    ExpectedCalleeSaved(Reg),
    DuplicateCalleeSaved(Reg),
}

impl fmt::Display for MachineEvalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingReturn => {
                formatter.write_str("machine function has no return instruction")
            }
            Self::MissingArgument { index, available } => write!(
                formatter,
                "argument {index} is missing; only {available} arguments were provided"
            ),
            Self::StackSlotOutOfBounds { slot, stack_slots } => write!(
                formatter,
                "stack slot {} is outside the allocated {} slots",
                slot.0, stack_slots
            ),
            Self::SetCcWithoutCmp => {
                formatter.write_str("setcc instruction has no preceding cmp instruction")
            }
            Self::DivisionByZero => formatter.write_str("signed division by zero"),
            Self::DivisionOverflow => {
                formatter.write_str("signed division overflow (i64::MIN / -1)")
            }
            Self::UnknownLabel(label) => write!(formatter, "unknown machine label {label}"),
            Self::InstructionLimitExceeded => {
                formatter.write_str("machine evaluation exceeded the instruction limit")
            }
            Self::UnsupportedCall(target) => {
                write!(formatter, "call to {target} requires a program evaluator")
            }
            Self::InvalidDivisor(reg) => {
                write!(formatter, "{reg} cannot be used as an idiv divisor")
            }
            Self::ImmediateDivision => {
                formatter.write_str("x86 does not support immediate division")
            }
            Self::ReservedRegister(reg) => {
                write!(formatter, "{reg} is reserved for frame management")
            }
            Self::ExpectedCalleeSaved(reg) => {
                write!(formatter, "{reg} is not a callee-saved register")
            }
            Self::DuplicateCalleeSaved(reg) => {
                write!(formatter, "duplicate callee-saved register {reg}")
            }
        }
    }
}

impl Error for MachineEvalError {}

pub fn evaluate(function: &MachineFunction, arguments: &[i64]) -> Result<i64, MachineEvalError> {
    execute(function, arguments).map(|execution| execution.value)
}

pub fn interpret(function: &MachineFunction, arguments: &[i64]) -> Result<i64, MachineEvalError> {
    evaluate(function, arguments)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Execution {
    value: i64,
    registers: [i64; REGISTER_COUNT],
}

fn execute(function: &MachineFunction, arguments: &[i64]) -> Result<Execution, MachineEvalError> {
    validate_function(function)?;

    let mut registers = [0; REGISTER_COUNT];
    for (index, register) in Reg::ARGUMENTS.into_iter().enumerate() {
        if let Some(value) = arguments.get(index) {
            registers[register_index(register)] = *value;
        }
    }
    for (register, value) in Reg::CALLEE_SAVED.into_iter().zip(CALLEE_SAVED_INITIAL) {
        registers[register_index(register)] = value;
    }

    let saved = function
        .callee_saved
        .iter()
        .map(|register| (*register, registers[register_index(*register)]))
        .collect::<Vec<_>>();
    let mut machine = Machine {
        registers,
        slots: vec![0; function.stack_slots as usize],
        comparison: None,
        saved,
    };

    let labels = function
        .instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| match instruction {
            MachineInstr::Label(label) => Some((*label, index)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut pc = 0usize;
    let mut executed = 0usize;
    while pc < function.instructions.len() {
        executed += 1;
        if executed > 1_000_000 {
            return Err(MachineEvalError::InstructionLimitExceeded);
        }
        let instruction = function.instructions[pc];
        pc += 1;
        match instruction {
            MachineInstr::LoadArg { dst, index } => {
                let argument = arguments
                    .get(index)
                    .ok_or(MachineEvalError::MissingArgument {
                        index,
                        available: arguments.len(),
                    })?;
                let value = if index < Reg::ARGUMENTS.len() {
                    machine.reg(Reg::ARGUMENTS[index])
                } else {
                    *argument
                };
                machine.set_reg(dst, value);
            }
            MachineInstr::MovImm { dst, value } => machine.set_reg(dst, value),
            MachineInstr::Mov { dst, src } => {
                let value = machine.reg(src);
                machine.set_reg(dst, value);
            }
            MachineInstr::Neg { dst } => {
                let value = machine.reg(dst).wrapping_neg();
                machine.set_reg(dst, value);
                machine.comparison = None;
            }
            MachineInstr::Not { dst } => {
                let value = !machine.reg(dst);
                machine.set_reg(dst, value);
            }
            MachineInstr::Add { dst, src } => {
                let value = machine
                    .reg(dst)
                    .wrapping_add(machine.operand(src, function.stack_slots)?);
                machine.set_reg(dst, value);
                machine.comparison = None;
            }
            MachineInstr::Sub { dst, src } => {
                let value = machine
                    .reg(dst)
                    .wrapping_sub(machine.operand(src, function.stack_slots)?);
                machine.set_reg(dst, value);
                machine.comparison = None;
            }
            MachineInstr::IMul { dst, src } => {
                let value = machine
                    .reg(dst)
                    .wrapping_mul(machine.operand(src, function.stack_slots)?);
                machine.set_reg(dst, value);
                machine.comparison = None;
            }
            MachineInstr::And { dst, src } => {
                let value = machine.reg(dst) & machine.operand(src, function.stack_slots)?;
                machine.set_reg(dst, value);
                machine.comparison = None;
            }
            MachineInstr::Or { dst, src } => {
                let value = machine.reg(dst) | machine.operand(src, function.stack_slots)?;
                machine.set_reg(dst, value);
                machine.comparison = None;
            }
            MachineInstr::Xor { dst, src } => {
                let value = machine.reg(dst) ^ machine.operand(src, function.stack_slots)?;
                machine.set_reg(dst, value);
                machine.comparison = None;
            }
            MachineInstr::Shl { dst } => {
                let count = machine.reg(Reg::Rcx) as u32 & 63;
                let value = machine.reg(dst).wrapping_shl(count);
                machine.set_reg(dst, value);
                if count != 0 {
                    machine.comparison = None;
                }
            }
            MachineInstr::Shr { dst } => {
                let count = machine.reg(Reg::Rcx) as u32 & 63;
                let value = machine.reg(dst).wrapping_shr(count);
                machine.set_reg(dst, value);
                if count != 0 {
                    machine.comparison = None;
                }
            }
            MachineInstr::LShr { dst } => {
                let count = machine.reg(Reg::Rcx) as u32 & 63;
                let value = (machine.reg(dst) as u64).wrapping_shr(count);
                machine.set_reg(dst, value as i64);
                if count != 0 {
                    machine.comparison = None;
                }
            }
            MachineInstr::ShlImm { dst, count } => {
                let value = machine.reg(dst).wrapping_shl(count as u32);
                machine.set_reg(dst, value);
                if count & 63 != 0 {
                    machine.comparison = None;
                }
            }
            MachineInstr::ShrImm { dst, count } => {
                let value = machine.reg(dst).wrapping_shr(count as u32);
                machine.set_reg(dst, value);
                if count & 63 != 0 {
                    machine.comparison = None;
                }
            }
            MachineInstr::LShrImm { dst, count } => {
                let value = (machine.reg(dst) as u64).wrapping_shr(count as u32);
                machine.set_reg(dst, value as i64);
                if count & 63 != 0 {
                    machine.comparison = None;
                }
            }
            MachineInstr::Cqo => {
                let sign = if machine.reg(Reg::Rax) < 0 { -1 } else { 0 };
                machine.set_reg(Reg::Rdx, sign);
            }
            MachineInstr::IDiv { src } => {
                let lhs = machine.reg(Reg::Rax);
                let rhs = machine.operand(src, function.stack_slots)?;
                if rhs == 0 {
                    return Err(MachineEvalError::DivisionByZero);
                }
                if lhs == i64::MIN && rhs == -1 {
                    return Err(MachineEvalError::DivisionOverflow);
                }
                machine.set_reg(Reg::Rax, lhs / rhs);
                machine.set_reg(Reg::Rdx, lhs % rhs);
                machine.comparison = None;
            }
            MachineInstr::UDiv { src } => {
                let lhs = machine.reg(Reg::Rax) as u64;
                let rhs = machine.operand(src, function.stack_slots)? as u64;
                if rhs == 0 {
                    return Err(MachineEvalError::DivisionByZero);
                }
                machine.set_reg(Reg::Rax, (lhs / rhs) as i64);
                machine.set_reg(Reg::Rdx, (lhs % rhs) as i64);
                machine.comparison = None;
            }
            MachineInstr::Cmp { lhs, rhs } => {
                machine.comparison = Some((
                    machine.reg(lhs),
                    machine.operand(rhs, function.stack_slots)?,
                ));
            }
            MachineInstr::Test { reg } => {
                machine.comparison = Some((machine.reg(reg), 0));
            }
            MachineInstr::SetCc { condition, dst } => {
                let (lhs, rhs) = machine
                    .comparison
                    .ok_or(MachineEvalError::SetCcWithoutCmp)?;
                machine.set_reg(dst, i64::from(evaluate_condition(condition, lhs, rhs)));
            }
            MachineInstr::Load { dst, slot } => {
                let value = machine.slot(slot, function.stack_slots)?;
                machine.set_reg(dst, value);
            }
            MachineInstr::Store { slot, src } => {
                let value = machine.reg(src);
                *machine.slot_mut(slot, function.stack_slots)? = value;
            }
            MachineInstr::StoreArg { .. } => {}
            MachineInstr::Label(_) => {}
            MachineInstr::Jmp(target) => {
                pc = *labels
                    .get(&target)
                    .ok_or(MachineEvalError::UnknownLabel(target))?;
            }
            MachineInstr::Jcc { condition, target } => {
                let (lhs, rhs) = machine
                    .comparison
                    .ok_or(MachineEvalError::SetCcWithoutCmp)?;
                if evaluate_condition(condition, lhs, rhs) {
                    pc = *labels
                        .get(&target)
                        .ok_or(MachineEvalError::UnknownLabel(target))?;
                }
            }
            MachineInstr::Call { target } => {
                return Err(MachineEvalError::UnsupportedCall(target));
            }
            MachineInstr::Ret => {
                for index in 0..machine.saved.len() {
                    let (register, value) = machine.saved[index];
                    machine.set_reg(register, value);
                }
                return Ok(Execution {
                    value: machine.reg(Reg::RETURN),
                    registers: machine.registers,
                });
            }
        }
    }

    Err(MachineEvalError::MissingReturn)
}

struct Machine {
    registers: [i64; REGISTER_COUNT],
    slots: Vec<i64>,
    comparison: Option<(i64, i64)>,
    saved: Vec<(Reg, i64)>,
}

impl Machine {
    fn reg(&self, register: Reg) -> i64 {
        self.registers[register_index(register)]
    }

    fn set_reg(&mut self, register: Reg, value: i64) {
        self.registers[register_index(register)] = value;
    }

    fn slot(&self, slot: StackSlot, stack_slots: u32) -> Result<i64, MachineEvalError> {
        self.slots
            .get(slot.0 as usize)
            .copied()
            .ok_or(MachineEvalError::StackSlotOutOfBounds { slot, stack_slots })
    }

    fn slot_mut(
        &mut self,
        slot: StackSlot,
        stack_slots: u32,
    ) -> Result<&mut i64, MachineEvalError> {
        self.slots
            .get_mut(slot.0 as usize)
            .ok_or(MachineEvalError::StackSlotOutOfBounds { slot, stack_slots })
    }

    fn operand(&self, operand: Operand, stack_slots: u32) -> Result<i64, MachineEvalError> {
        match operand {
            Operand::Reg(register) => Ok(self.reg(register)),
            Operand::Stack(slot) => self.slot(slot, stack_slots),
            Operand::Imm(value) => Ok(value as i64),
        }
    }
}

fn evaluate_condition(condition: Condition, lhs: i64, rhs: i64) -> bool {
    match condition {
        Condition::Eq => lhs == rhs,
        Condition::NotEq => lhs != rhs,
        Condition::SignedLess => lhs < rhs,
        Condition::SignedLessEq => lhs <= rhs,
        Condition::SignedGreater => lhs > rhs,
        Condition::SignedGreaterEq => lhs >= rhs,
        Condition::UnsignedBelow => (lhs as u64) < rhs as u64,
        Condition::UnsignedBelowEq => (lhs as u64) <= rhs as u64,
        Condition::UnsignedAbove => (lhs as u64) > rhs as u64,
        Condition::UnsignedAboveEq => (lhs as u64) >= rhs as u64,
    }
}

fn validate_function(function: &MachineFunction) -> Result<(), MachineEvalError> {
    for (index, register) in function.callee_saved.iter().copied().enumerate() {
        if is_reserved(register) {
            return Err(MachineEvalError::ReservedRegister(register));
        }
        if !matches!(
            register,
            Reg::Rbx | Reg::R12 | Reg::R13 | Reg::R14 | Reg::R15
        ) {
            return Err(MachineEvalError::ExpectedCalleeSaved(register));
        }
        if function.callee_saved[..index].contains(&register) {
            return Err(MachineEvalError::DuplicateCalleeSaved(register));
        }
    }
    if !function.instructions.contains(&MachineInstr::Ret) {
        return Err(MachineEvalError::MissingReturn);
    }
    for instruction in &function.instructions {
        validate_instruction(*instruction, function.stack_slots)?;
    }
    Ok(())
}

fn validate_instruction(
    instruction: MachineInstr,
    stack_slots: u32,
) -> Result<(), MachineEvalError> {
    use MachineInstr::*;

    let validate_reg = |register| {
        if is_reserved(register) {
            Err(MachineEvalError::ReservedRegister(register))
        } else {
            Ok(())
        }
    };
    let validate_operand = |operand| match operand {
        Operand::Reg(register) => validate_reg(register),
        Operand::Stack(slot) => validate_slot(slot, stack_slots),
        Operand::Imm(_) => Ok(()),
    };

    match instruction {
        LoadArg { dst, .. }
        | MovImm { dst, .. }
        | Neg { dst }
        | Not { dst }
        | Shl { dst }
        | Shr { dst }
        | LShr { dst }
        | ShlImm { dst, .. }
        | ShrImm { dst, .. }
        | LShrImm { dst, .. }
        | SetCc { dst, .. } => validate_reg(dst),
        Mov { dst, src } => {
            validate_reg(dst)?;
            validate_reg(src)
        }
        Add { dst, src }
        | Sub { dst, src }
        | IMul { dst, src }
        | And { dst, src }
        | Or { dst, src }
        | Xor { dst, src } => {
            validate_reg(dst)?;
            validate_operand(src)
        }
        Cqo => Ok(()),
        IDiv { src } | UDiv { src } => {
            if matches!(src, Operand::Imm(_)) {
                return Err(MachineEvalError::ImmediateDivision);
            }
            validate_operand(src)?;
            if let Operand::Reg(reg @ (Reg::Rax | Reg::Rdx)) = src {
                Err(MachineEvalError::InvalidDivisor(reg))
            } else {
                Ok(())
            }
        }
        Cmp { lhs, rhs } => {
            validate_reg(lhs)?;
            validate_operand(rhs)
        }
        Test { reg } => validate_reg(reg),
        Load { dst, slot } => {
            validate_reg(dst)?;
            validate_slot(slot, stack_slots)
        }
        Store { slot, src } => {
            validate_slot(slot, stack_slots)?;
            validate_reg(src)
        }
        StoreArg { .. } => Ok(()),
        Label(_) | Jmp(_) | Jcc { .. } | Call { .. } => Ok(()),
        Ret => Ok(()),
    }
}

fn validate_slot(slot: StackSlot, stack_slots: u32) -> Result<(), MachineEvalError> {
    if slot.0 < stack_slots {
        Ok(())
    } else {
        Err(MachineEvalError::StackSlotOutOfBounds { slot, stack_slots })
    }
}

fn is_reserved(register: Reg) -> bool {
    matches!(register, Reg::Rbp | Reg::Rsp)
}

const fn register_index(register: Reg) -> usize {
    match register {
        Reg::Rax => 0,
        Reg::Rbx => 1,
        Reg::Rcx => 2,
        Reg::Rdx => 3,
        Reg::Rsi => 4,
        Reg::Rdi => 5,
        Reg::Rbp => 6,
        Reg::Rsp => 7,
        Reg::R8 => 8,
        Reg::R9 => 9,
        Reg::R10 => 10,
        Reg::R11 => 11,
        Reg::R12 => 12,
        Reg::R13 => 13,
        Reg::R14 => 14,
        Reg::R15 => 15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function(instructions: Vec<MachineInstr>) -> MachineFunction {
        MachineFunction {
            name: "test_fn".into(),
            instructions,
            stack_slots: 0,
            callee_saved: Vec::new(),
            symbols: Vec::new(),
        }
    }

    fn immediate(register: Reg, value: i64) -> MachineInstr {
        MachineInstr::MovImm {
            dst: register,
            value,
        }
    }

    fn compare(condition: Condition, lhs: i64, rhs: i64) -> Result<i64, MachineEvalError> {
        evaluate(
            &function(vec![
                immediate(Reg::Rax, lhs),
                immediate(Reg::Rcx, rhs),
                MachineInstr::Cmp {
                    lhs: Reg::Rax,
                    rhs: Operand::Reg(Reg::Rcx),
                },
                MachineInstr::SetCc {
                    condition,
                    dst: Reg::Rax,
                },
                MachineInstr::Ret,
            ]),
            &[],
        )
    }

    #[test]
    fn arithmetic_moves_and_wrapping_match_i64() {
        let cases = [
            (i64::MAX, 1, i64::MIN),
            (i64::MIN, 1, i64::MAX),
            (i64::MAX, 2, -2),
        ];
        for (index, (lhs, rhs, expected)) in cases.into_iter().enumerate() {
            let operation = match index {
                0 => MachineInstr::Add {
                    dst: Reg::Rax,
                    src: Operand::Reg(Reg::Rcx),
                },
                1 => MachineInstr::Sub {
                    dst: Reg::Rax,
                    src: Operand::Reg(Reg::Rcx),
                },
                2 => MachineInstr::IMul {
                    dst: Reg::Rax,
                    src: Operand::Reg(Reg::Rcx),
                },
                _ => unreachable!(),
            };
            assert_eq!(
                evaluate(
                    &function(vec![
                        immediate(Reg::Rdx, lhs),
                        MachineInstr::Mov {
                            dst: Reg::Rax,
                            src: Reg::Rdx,
                        },
                        immediate(Reg::Rcx, rhs),
                        operation,
                        MachineInstr::Ret,
                    ]),
                    &[],
                ),
                Ok(expected)
            );
        }
        assert_eq!(
            evaluate(
                &function(vec![
                    immediate(Reg::Rax, i64::MIN),
                    immediate(Reg::Rcx, -1),
                    MachineInstr::IMul {
                        dst: Reg::Rax,
                        src: Operand::Reg(Reg::Rcx),
                    },
                    MachineInstr::Ret,
                ]),
                &[],
            ),
            Ok(i64::MIN)
        );
    }

    #[test]
    fn signed_division_produces_quotient_remainder_and_traps() {
        let divide = |lhs, rhs| {
            function(vec![
                immediate(Reg::Rax, lhs),
                immediate(Reg::Rcx, rhs),
                MachineInstr::Cqo,
                MachineInstr::IDiv {
                    src: Operand::Reg(Reg::Rcx),
                },
                MachineInstr::Ret,
            ])
        };
        for (lhs, rhs, quotient, remainder) in [
            (-42, 4, -10, -2),
            (42, -4, -10, 2),
            (-42, -4, 10, -2),
            (i64::MIN, 1, i64::MIN, 0),
        ] {
            let execution = execute(&divide(lhs, rhs), &[]).unwrap();
            assert_eq!(execution.value, quotient);
            assert_eq!(execution.registers[register_index(Reg::Rdx)], remainder);
        }

        assert_eq!(
            evaluate(&divide(1, 0), &[]),
            Err(MachineEvalError::DivisionByZero)
        );
        assert_eq!(
            evaluate(&divide(i64::MIN, -1), &[]),
            Err(MachineEvalError::DivisionOverflow)
        );
        for divisor in [Reg::Rax, Reg::Rdx] {
            assert_eq!(
                evaluate(
                    &function(vec![
                        MachineInstr::IDiv {
                            src: Operand::Reg(divisor),
                        },
                        MachineInstr::Ret,
                    ]),
                    &[],
                ),
                Err(MachineEvalError::InvalidDivisor(divisor))
            );
        }
    }

    #[test]
    fn spill_slots_support_load_store_and_memory_operands() {
        let function = MachineFunction {
            instructions: vec![
                immediate(Reg::Rax, 40),
                MachineInstr::Store {
                    slot: StackSlot(0),
                    src: Reg::Rax,
                },
                immediate(Reg::Rcx, 2),
                MachineInstr::Store {
                    slot: StackSlot(1),
                    src: Reg::Rcx,
                },
                immediate(Reg::Rax, 0),
                MachineInstr::Load {
                    dst: Reg::Rax,
                    slot: StackSlot(0),
                },
                MachineInstr::Add {
                    dst: Reg::Rax,
                    src: Operand::Stack(StackSlot(1)),
                },
                MachineInstr::Ret,
            ],
            stack_slots: 2,
            ..function(Vec::new())
        };
        assert_eq!(evaluate(&function, &[]), Ok(42));
    }

    #[test]
    fn callee_saved_registers_have_stable_initial_values_and_are_restored() {
        let saved_registers = MachineFunction {
            instructions: vec![
                immediate(Reg::Rbx, 11),
                immediate(Reg::R12, 31),
                MachineInstr::Mov {
                    dst: Reg::Rax,
                    src: Reg::Rbx,
                },
                MachineInstr::Add {
                    dst: Reg::Rax,
                    src: Operand::Reg(Reg::R12),
                },
                MachineInstr::Ret,
            ],
            callee_saved: vec![Reg::Rbx, Reg::R12],
            ..function(Vec::new())
        };
        let execution = execute(&saved_registers, &[]).unwrap();
        assert_eq!(execution.value, 42);
        assert_eq!(
            execution.registers[register_index(Reg::Rbx)],
            CALLEE_SAVED_INITIAL[0]
        );
        assert_eq!(
            execution.registers[register_index(Reg::R12)],
            CALLEE_SAVED_INITIAL[2]
        );

        let initial = function(vec![
            MachineInstr::Mov {
                dst: Reg::Rax,
                src: Reg::R13,
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(
            evaluate(&initial, &[]),
            Ok(CALLEE_SAVED_INITIAL[3]),
            "callee-saved input values must not alias argument registers"
        );
    }

    #[test]
    fn all_conditions_produce_zero_extended_booleans() {
        let cases = [
            (Condition::Eq, 4, 4, 1),
            (Condition::NotEq, 4, 4, 0),
            (Condition::SignedLess, -1, 0, 1),
            (Condition::SignedLessEq, 1, 0, 0),
            (Condition::SignedGreater, 1, 0, 1),
            (Condition::SignedGreaterEq, -1, 0, 0),
            (Condition::UnsignedBelow, -1, 0, 0),
            (Condition::UnsignedBelowEq, -1, -1, 1),
            (Condition::UnsignedAbove, -1, 0, 1),
            (Condition::UnsignedAboveEq, 0, 1, 0),
        ];
        for (condition, lhs, rhs, expected) in cases {
            assert_eq!(compare(condition, lhs, rhs), Ok(expected), "{condition:?}");
        }
    }

    #[test]
    fn loads_register_and_stack_arguments_and_reports_missing_arguments() {
        let mut instructions = vec![MachineInstr::LoadArg {
            dst: Reg::Rax,
            index: 0,
        }];
        for index in 1..8 {
            instructions.extend([
                MachineInstr::LoadArg {
                    dst: Reg::R10,
                    index,
                },
                MachineInstr::Add {
                    dst: Reg::Rax,
                    src: Operand::Reg(Reg::R10),
                },
            ]);
        }
        instructions.push(MachineInstr::Ret);
        let arguments = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(
            evaluate(&function(instructions.clone()), &arguments),
            Ok(36)
        );
        assert_eq!(
            evaluate(&function(instructions), &arguments[..6]),
            Err(MachineEvalError::MissingArgument {
                index: 6,
                available: 6,
            })
        );

        let current_argument_register = function(vec![
            immediate(Reg::Rdi, 99),
            MachineInstr::LoadArg {
                dst: Reg::Rax,
                index: 0,
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(evaluate(&current_argument_register, &[1]), Ok(99));
        assert_eq!(
            evaluate(&current_argument_register, &[]),
            Err(MachineEvalError::MissingArgument {
                index: 0,
                available: 0,
            })
        );
    }

    #[test]
    fn malformed_functions_return_explicit_errors() {
        assert_eq!(
            evaluate(&function(Vec::new()), &[]),
            Err(MachineEvalError::MissingReturn)
        );
        assert_eq!(
            evaluate(
                &function(vec![
                    MachineInstr::Load {
                        dst: Reg::Rax,
                        slot: StackSlot(1),
                    },
                    MachineInstr::Ret,
                ]),
                &[],
            ),
            Err(MachineEvalError::StackSlotOutOfBounds {
                slot: StackSlot(1),
                stack_slots: 0,
            })
        );
        assert_eq!(
            evaluate(
                &function(vec![
                    MachineInstr::SetCc {
                        condition: Condition::Eq,
                        dst: Reg::Rax,
                    },
                    MachineInstr::Ret,
                ]),
                &[],
            ),
            Err(MachineEvalError::SetCcWithoutCmp)
        );
        assert_eq!(
            evaluate(
                &function(vec![immediate(Reg::Rsp, 1), MachineInstr::Ret]),
                &[],
            ),
            Err(MachineEvalError::ReservedRegister(Reg::Rsp))
        );

        let caller_saved = MachineFunction {
            callee_saved: vec![Reg::Rax],
            ..function(vec![MachineInstr::Ret])
        };
        assert_eq!(
            evaluate(&caller_saved, &[]),
            Err(MachineEvalError::ExpectedCalleeSaved(Reg::Rax))
        );
        let duplicate = MachineFunction {
            callee_saved: vec![Reg::Rbx, Reg::Rbx],
            ..function(vec![MachineInstr::Ret])
        };
        assert_eq!(
            evaluate(&duplicate, &[]),
            Err(MachineEvalError::DuplicateCalleeSaved(Reg::Rbx))
        );
    }
}
