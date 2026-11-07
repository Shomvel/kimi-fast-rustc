//! x86_64 machine-code emission for the native instruction IR.

use crate::x86::{Condition, MachineFunction, MachineInstr, Operand, Reg, StackSlot};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallRelocation {
    pub offset: u64,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncodeError {
    ReservedRegister(Reg),
    ExpectedCalleeSaved(Reg),
    DuplicateCalleeSaved(Reg),
    InvalidDivisor(Reg),
    InvalidCallTarget(u32),
    StackSlotOutOfBounds { slot: StackSlot, stack_slots: u32 },
    ArgumentOffsetOverflow { index: usize },
    FrameSizeOverflow,
    MissingReturn,
    UnknownLabel(u32),
    DuplicateLabel(u32),
    RelocationOverflow(u32),
    CodeSizeOverflow,
    AllocationFailed,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservedRegister(reg) => {
                write!(formatter, "{reg} is reserved for frame management")
            }
            Self::ExpectedCalleeSaved(reg) => {
                write!(formatter, "{reg} is not a callee-saved register")
            }
            Self::DuplicateCalleeSaved(reg) => {
                write!(formatter, "duplicate callee-saved register {reg}")
            }
            Self::InvalidDivisor(reg) => {
                write!(formatter, "{reg} cannot be used as an idiv divisor")
            }
            Self::InvalidCallTarget(target) => {
                write!(formatter, "call target symbol index {target} is invalid")
            }
            Self::StackSlotOutOfBounds { slot, stack_slots } => write!(
                formatter,
                "stack slot {} is outside the allocated {} slots",
                slot.0, stack_slots
            ),
            Self::ArgumentOffsetOverflow { index } => {
                write!(
                    formatter,
                    "stack argument offset cannot be encoded for argument {index}"
                )
            }
            Self::FrameSizeOverflow => formatter.write_str("stack frame cannot be encoded"),
            Self::MissingReturn => {
                formatter.write_str("machine function has no return instruction")
            }
            Self::UnknownLabel(label) => write!(formatter, "unknown machine label {label}"),
            Self::DuplicateLabel(label) => write!(formatter, "duplicate machine label {label}"),
            Self::RelocationOverflow(label) => {
                write!(formatter, "jump to label {label} exceeds the rel32 range")
            }
            Self::CodeSizeOverflow => formatter.write_str("machine-code size overflows usize"),
            Self::AllocationFailed => {
                formatter.write_str("could not allocate the machine-code buffer")
            }
        }
    }
}

impl Error for EncodeError {}

impl MachineFunction {
    pub fn emit_machine_code(&self) -> Result<Vec<u8>, EncodeError> {
        emit_machine_code(self)
    }
}

/// Encodes one function as position-independent bytes suitable for ELF `.text`.
/// Instructions, including every return sequence, retain their original order.
pub fn emit_machine_code(function: &MachineFunction) -> Result<Vec<u8>, EncodeError> {
    encode_with_relocations(function).map(|(code, _)| code)
}

pub fn encode_with_relocations(
    function: &MachineFunction,
) -> Result<(Vec<u8>, Vec<CallRelocation>), EncodeError> {
    let frame_size = validate_function(function)?;
    let return_count = function
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction, MachineInstr::Ret))
        .count();
    let save_restore_bytes = function
        .callee_saved
        .len()
        .checked_mul(7)
        .and_then(|size| size.checked_mul(return_count + 1))
        .ok_or(EncodeError::CodeSizeOverflow)?;
    let capacity = 11_usize
        .checked_add(save_restore_bytes)
        .and_then(|size| {
            function
                .instructions
                .len()
                .checked_mul(10)
                .and_then(|instruction_bytes| size.checked_add(instruction_bytes))
        })
        .ok_or(EncodeError::CodeSizeOverflow)?;
    let mut encoder = Encoder {
        bytes: Vec::new(),
        stack_slots: function.stack_slots,
        callee_saved: &function.callee_saved,
        symbols: &function.symbols,
        calls: Vec::new(),
        labels: BTreeMap::new(),
        fixups: Vec::new(),
    };
    encoder
        .bytes
        .try_reserve_exact(capacity)
        .map_err(|_| EncodeError::AllocationFailed)?;

    encoder.prologue(frame_size)?;
    for instruction in &function.instructions {
        encoder.instruction(*instruction)?;
    }
    encoder.resolve_labels()?;
    Ok((encoder.bytes, encoder.calls))
}

fn validate_function(function: &MachineFunction) -> Result<u32, EncodeError> {
    for (index, reg) in function.callee_saved.iter().copied().enumerate() {
        if matches!(reg, Reg::Rbp | Reg::Rsp) {
            return Err(EncodeError::ReservedRegister(reg));
        }
        if !matches!(reg, Reg::Rbx | Reg::R12 | Reg::R13 | Reg::R14 | Reg::R15) {
            return Err(EncodeError::ExpectedCalleeSaved(reg));
        }
        if function.callee_saved[..index].contains(&reg) {
            return Err(EncodeError::DuplicateCalleeSaved(reg));
        }
    }
    if !function.instructions.contains(&MachineInstr::Ret) {
        return Err(EncodeError::MissingReturn);
    }

    let saved_bytes = function
        .callee_saved
        .len()
        .checked_mul(8)
        .ok_or(EncodeError::FrameSizeOverflow)?;
    let spill_bytes = (function.stack_slots as usize)
        .checked_mul(8)
        .ok_or(EncodeError::FrameSizeOverflow)?;
    let frame_size = spill_bytes
        .checked_add(saved_bytes)
        .and_then(|size| size.checked_add(15))
        .map(|size| size & !15)
        .ok_or(EncodeError::FrameSizeOverflow)?;
    let frame_size = u32::try_from(frame_size).map_err(|_| EncodeError::FrameSizeOverflow)?;
    if frame_size > i32::MAX as u32 {
        return Err(EncodeError::FrameSizeOverflow);
    }

    for instruction in &function.instructions {
        validate_instruction(*instruction, function.stack_slots, &function.symbols)?;
    }
    Ok(frame_size)
}

fn validate_instruction(
    instruction: MachineInstr,
    stack_slots: u32,
    symbols: &[String],
) -> Result<(), EncodeError> {
    use MachineInstr::*;

    fn validate_reg(reg: Reg) -> Result<(), EncodeError> {
        if matches!(reg, Reg::Rbp | Reg::Rsp) {
            Err(EncodeError::ReservedRegister(reg))
        } else {
            Ok(())
        }
    }

    fn validate_operand(operand: Operand, stack_slots: u32) -> Result<(), EncodeError> {
        match operand {
            Operand::Reg(reg) => validate_reg(reg),
            Operand::Stack(slot) => validate_slot(slot, stack_slots),
        }
    }

    match instruction {
        LoadArg { dst, index } => {
            validate_reg(dst)?;
            if index >= Reg::ARGUMENTS.len() {
                incoming_arg_offset(index)?;
            }
        }
        MovImm { dst, .. }
        | Neg { dst }
        | Not { dst }
        | Shl { dst }
        | Shr { dst }
        | LShr { dst }
        | ShlImm { dst, .. }
        | ShrImm { dst, .. }
        | LShrImm { dst, .. }
        | SetCc { dst, .. } => validate_reg(dst)?,
        Mov { dst, src } => {
            validate_reg(dst)?;
            validate_reg(src)?;
        }
        Add { dst, src }
        | Sub { dst, src }
        | IMul { dst, src }
        | And { dst, src }
        | Or { dst, src }
        | Xor { dst, src } => {
            validate_reg(dst)?;
            validate_operand(src, stack_slots)?;
        }
        Cqo => {}
        IDiv { src } | UDiv { src } => {
            validate_operand(src, stack_slots)?;
            if let Operand::Reg(reg @ (Reg::Rax | Reg::Rdx)) = src {
                return Err(EncodeError::InvalidDivisor(reg));
            }
        }
        Cmp { lhs, rhs } => {
            validate_reg(lhs)?;
            validate_operand(rhs, stack_slots)?;
        }
        Load { dst, slot } => {
            validate_reg(dst)?;
            validate_slot(slot, stack_slots)?;
        }
        Store { slot, src } => {
            validate_slot(slot, stack_slots)?;
            validate_reg(src)?;
        }
        StoreArg { src, index } => {
            validate_reg(src)?;
            if index < Reg::ARGUMENTS.len() {
                return Err(EncodeError::ArgumentOffsetOverflow { index });
            }
            incoming_arg_offset(index)?;
        }
        Label(_) | Jmp(_) | Jcc { .. } => {}
        Call { target } => {
            if target as usize >= symbols.len() {
                return Err(EncodeError::InvalidCallTarget(target));
            }
        }
        Ret => {}
    }
    Ok(())
}

fn validate_slot(slot: StackSlot, stack_slots: u32) -> Result<(), EncodeError> {
    if slot.0 < stack_slots {
        Ok(())
    } else {
        Err(EncodeError::StackSlotOutOfBounds { slot, stack_slots })
    }
}

fn spill_offset(slot: StackSlot) -> Result<i32, EncodeError> {
    let offset = (u64::from(slot.0) + 1)
        .checked_mul(8)
        .and_then(|offset| i32::try_from(offset).ok())
        .ok_or(EncodeError::FrameSizeOverflow)?;
    Ok(-offset)
}

fn saved_offset(stack_slots: u32, index: usize) -> Result<i32, EncodeError> {
    let offset = (stack_slots as usize)
        .checked_add(index)
        .and_then(|slot| slot.checked_add(1))
        .and_then(|slot| slot.checked_mul(8))
        .and_then(|offset| i32::try_from(offset).ok())
        .ok_or(EncodeError::FrameSizeOverflow)?;
    Ok(-offset)
}

fn incoming_arg_offset(index: usize) -> Result<i32, EncodeError> {
    index
        .checked_sub(Reg::ARGUMENTS.len())
        .and_then(|stack_index| stack_index.checked_mul(8))
        .and_then(|offset| offset.checked_add(16))
        .and_then(|offset| i32::try_from(offset).ok())
        .ok_or(EncodeError::ArgumentOffsetOverflow { index })
}

struct Encoder<'a> {
    bytes: Vec<u8>,
    stack_slots: u32,
    callee_saved: &'a [Reg],
    symbols: &'a [String],
    calls: Vec<CallRelocation>,
    labels: BTreeMap<u32, usize>,
    fixups: Vec<(usize, u32)>,
}

impl Encoder<'_> {
    fn prologue(&mut self, frame_size: u32) -> Result<(), EncodeError> {
        self.bytes.extend_from_slice(&[0x55, 0x48, 0x89, 0xe5]);
        if frame_size != 0 {
            self.bytes.extend_from_slice(&[0x48, 0x81, 0xec]);
            self.i32(frame_size as i32);
        }
        for (index, reg) in self.callee_saved.iter().copied().enumerate() {
            let offset = saved_offset(self.stack_slots, index)?;
            self.memory_reg(0x89, reg, offset);
        }
        Ok(())
    }

    fn instruction(&mut self, instruction: MachineInstr) -> Result<(), EncodeError> {
        use MachineInstr::*;

        match instruction {
            LoadArg { dst, index } => {
                if let Some(src) = Reg::ARGUMENTS.get(index) {
                    self.reg_reg(0x89, dst, *src);
                } else {
                    self.memory_reg(0x8b, dst, incoming_arg_offset(index)?);
                }
            }
            MovImm { dst, value } => {
                self.rex(true, None, Some(dst), false);
                if i32::try_from(value).is_ok() {
                    self.bytes.push(0xc7);
                    self.modrm(3, Reg::Rax, dst.low_code());
                    self.i32(value as i32);
                } else {
                    self.bytes.push(0xb8 + dst.low_code());
                    self.bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            Mov { dst, src } => self.reg_reg(0x89, dst, src),
            Neg { dst } => {
                self.rex(true, None, Some(dst), false);
                self.bytes.push(0xf7);
                self.modrm(3, Reg::Rbx, dst.low_code());
            }
            Not { dst } => {
                self.rex(true, None, Some(dst), false);
                self.bytes.push(0xf7);
                self.modrm(3, Reg::Rdx, dst.low_code());
            }
            Add { dst, src } => self.binary(0x01, 0x03, dst, src)?,
            Sub { dst, src } => self.binary(0x29, 0x2b, dst, src)?,
            And { dst, src } => self.binary(0x21, 0x23, dst, src)?,
            Or { dst, src } => self.binary(0x09, 0x0b, dst, src)?,
            Xor { dst, src } => self.binary(0x31, 0x33, dst, src)?,
            Shl { dst } => {
                self.rex(true, None, Some(dst), false);
                self.bytes.push(0xd3);
                self.modrm(3, Reg::Rsp, dst.low_code());
            }
            Shr { dst } => {
                self.rex(true, None, Some(dst), false);
                self.bytes.push(0xd3);
                self.modrm(3, Reg::Rdi, dst.low_code());
            }
            LShr { dst } => {
                self.rex(true, None, Some(dst), false);
                self.bytes.push(0xd3);
                self.modrm(3, Reg::Rbp, dst.low_code());
            }
            ShlImm { dst, count } => {
                self.rex(true, None, Some(dst), false);
                self.bytes.push(0xc1);
                self.modrm(3, Reg::Rsp, dst.low_code());
                self.bytes.push(count);
            }
            ShrImm { dst, count } => {
                self.rex(true, None, Some(dst), false);
                self.bytes.push(0xc1);
                self.modrm(3, Reg::Rdi, dst.low_code());
                self.bytes.push(count);
            }
            LShrImm { dst, count } => {
                self.rex(true, None, Some(dst), false);
                self.bytes.push(0xc1);
                self.modrm(3, Reg::Rbp, dst.low_code());
                self.bytes.push(count);
            }
            IMul { dst, src } => match src {
                Operand::Reg(src) => {
                    self.rex(true, Some(dst), Some(src), false);
                    self.bytes.extend_from_slice(&[0x0f, 0xaf]);
                    self.modrm(3, dst, src.low_code());
                }
                Operand::Stack(slot) => {
                    self.rex(true, Some(dst), None, false);
                    self.bytes.extend_from_slice(&[0x0f, 0xaf]);
                    self.stack_memory(dst, spill_offset(slot)?);
                }
            },
            Cqo => self.bytes.extend_from_slice(&[0x48, 0x99]),
            IDiv { src } => match src {
                Operand::Reg(src) => {
                    self.rex(true, None, Some(src), false);
                    self.bytes.push(0xf7);
                    self.modrm(3, Reg::Rdi, src.low_code());
                }
                Operand::Stack(slot) => {
                    self.bytes.push(0x48);
                    self.bytes.push(0xf7);
                    self.stack_memory(Reg::Rdi, spill_offset(slot)?);
                }
            },
            UDiv { src } => match src {
                Operand::Reg(src) => {
                    self.rex(true, None, Some(src), false);
                    self.bytes.push(0xf7);
                    self.modrm(3, Reg::Rsi, src.low_code());
                }
                Operand::Stack(slot) => {
                    self.bytes.push(0x48);
                    self.bytes.push(0xf7);
                    self.stack_memory(Reg::Rsi, spill_offset(slot)?);
                }
            },
            Cmp { lhs, rhs } => self.binary(0x39, 0x3b, lhs, rhs)?,
            SetCc { condition, dst } => {
                self.rex(false, None, Some(dst), dst.code() >= 4);
                self.bytes
                    .extend_from_slice(&[0x0f, 0x90 + condition_code(condition)]);
                self.modrm(3, Reg::Rax, dst.low_code());
                self.rex(false, Some(dst), Some(dst), dst.code() >= 4);
                self.bytes.extend_from_slice(&[0x0f, 0xb6]);
                self.modrm(3, dst, dst.low_code());
            }
            Load { dst, slot } => self.memory_reg(0x8b, dst, spill_offset(slot)?),
            Store { slot, src } => self.memory_reg(0x89, src, spill_offset(slot)?),
            StoreArg { src, index } => {
                if index < Reg::ARGUMENTS.len() {
                    return Err(EncodeError::ArgumentOffsetOverflow { index });
                }
                self.rex(true, Some(src), None, false);
                self.bytes.push(0x89);
                self.stack_memory_rsp(src, ((index - Reg::ARGUMENTS.len()) * 8) as i32);
            }
            Call { target } => {
                self.calls.push(CallRelocation {
                    offset: (self.bytes.len() + 1) as u64,
                    target: self.symbols[target as usize].clone(),
                });
                self.bytes.extend_from_slice(&[0xe8, 0, 0, 0, 0]);
            }
            Label(label) => {
                let offset = self.bytes.len();
                if self.labels.insert(label, offset).is_some() {
                    return Err(EncodeError::DuplicateLabel(label));
                }
            }
            Jmp(target) => {
                self.bytes.push(0xe9);
                self.fixups.push((self.bytes.len(), target));
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
            }
            Jcc { condition, target } => {
                self.bytes
                    .extend_from_slice(&[0x0f, 0x80 + condition_code(condition)]);
                self.fixups.push((self.bytes.len(), target));
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
            }
            Ret => {
                for (index, reg) in self.callee_saved.iter().copied().enumerate() {
                    let offset = saved_offset(self.stack_slots, index)?;
                    self.memory_reg(0x8b, reg, offset);
                }
                self.bytes.extend_from_slice(&[0xc9, 0xc3]);
            }
        }
        Ok(())
    }

    fn resolve_labels(&mut self) -> Result<(), EncodeError> {
        for (offset, target) in self.fixups.iter().copied() {
            let destination = self
                .labels
                .get(&target)
                .copied()
                .ok_or(EncodeError::UnknownLabel(target))?;
            let displacement = destination as i128 - (offset as i128 + 4);
            let displacement =
                i32::try_from(displacement).map_err(|_| EncodeError::RelocationOverflow(target))?;
            self.bytes[offset..offset + 4].copy_from_slice(&displacement.to_le_bytes());
        }
        Ok(())
    }

    fn binary(
        &mut self,
        reg_opcode: u8,
        memory_opcode: u8,
        dst: Reg,
        src: Operand,
    ) -> Result<(), EncodeError> {
        match src {
            Operand::Reg(src) => self.reg_reg(reg_opcode, dst, src),
            Operand::Stack(slot) => self.memory_reg(memory_opcode, dst, spill_offset(slot)?),
        }
        Ok(())
    }

    fn reg_reg(&mut self, opcode: u8, dst: Reg, src: Reg) {
        self.rex(true, Some(src), Some(dst), false);
        self.bytes.push(opcode);
        self.modrm(3, src, dst.low_code());
    }

    fn memory_reg(&mut self, opcode: u8, reg: Reg, offset: i32) {
        self.rex(true, Some(reg), None, false);
        self.bytes.push(opcode);
        self.stack_memory(reg, offset);
    }

    fn stack_memory(&mut self, reg: Reg, offset: i32) {
        self.modrm(2, reg, Reg::Rbp.low_code());
        self.i32(offset);
    }

    fn stack_memory_rsp(&mut self, reg: Reg, offset: i32) {
        self.modrm(2, reg, Reg::Rsp.low_code());
        self.bytes.push(0x24);
        self.i32(offset);
    }

    fn rex(&mut self, w: bool, reg: Option<Reg>, base: Option<Reg>, force: bool) {
        let mut rex = if w { 0x48 } else { 0x40 };
        if reg.is_some_and(|reg| reg.code() >= 8) {
            rex |= 0x04;
        }
        if base.is_some_and(|reg| reg.code() >= 8) {
            rex |= 0x01;
        }
        if rex != 0x40 || force {
            self.bytes.push(rex);
        }
    }

    fn modrm(&mut self, mode: u8, reg: Reg, rm: u8) {
        self.bytes.push((mode << 6) | (reg.low_code() << 3) | rm);
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
}

trait RegCode: Sized {
    fn code(self) -> u8;

    fn low_code(self) -> u8 {
        self.code() & 7
    }
}

impl RegCode for Reg {
    fn code(self) -> u8 {
        match self {
            Reg::Rax => 0,
            Reg::Rcx => 1,
            Reg::Rdx => 2,
            Reg::Rbx => 3,
            Reg::Rsp => 4,
            Reg::Rbp => 5,
            Reg::Rsi => 6,
            Reg::Rdi => 7,
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
}

fn condition_code(condition: Condition) -> u8 {
    match condition {
        Condition::Eq => 0x4,
        Condition::NotEq => 0x5,
        Condition::SignedLess => 0xc,
        Condition::SignedLessEq => 0xe,
        Condition::SignedGreater => 0xf,
        Condition::SignedGreaterEq => 0xd,
        Condition::UnsignedBelow => 0x2,
        Condition::UnsignedBelowEq => 0x6,
        Condition::UnsignedAbove => 0x7,
        Condition::UnsignedAboveEq => 0x3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elf::{FunctionSymbol, write_object};
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn function(instructions: Vec<MachineInstr>) -> MachineFunction {
        MachineFunction {
            name: "test_fn".into(),
            instructions,
            stack_slots: 0,
            callee_saved: Vec::new(),
            symbols: Vec::new(),
        }
    }

    #[test]
    fn known_vector_for_arguments_immediates_arithmetic_and_return() {
        let code = function(vec![
            MachineInstr::MovImm {
                dst: Reg::Rax,
                value: 0x1122_3344_5566_7788,
            },
            MachineInstr::LoadArg {
                dst: Reg::Rcx,
                index: 0,
            },
            MachineInstr::Add {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::LoadArg {
                dst: Reg::R8,
                index: 6,
            },
            MachineInstr::Sub {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::R8),
            },
            MachineInstr::IMul {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::Mov {
                dst: Reg::R9,
                src: Reg::Rax,
            },
            MachineInstr::Ret,
        ])
        .emit_machine_code()
        .unwrap();
        let expected = [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0xb8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
            0x48, 0x89, 0xf9, 0x48, 0x01, 0xc8, 0x4c, 0x8b, 0x85, 0x10, 0x00, 0x00, 0x00, 0x4c,
            0x29, 0xc0, 0x48, 0x0f, 0xaf, 0xc1, 0x49, 0x89, 0xc1, 0xc9, 0xc3,
        ];
        assert_eq!(code, expected);
    }

    #[test]
    fn known_vector_for_frame_memory_comparisons_and_callee_saved_registers() {
        let function = MachineFunction {
            name: "frame".into(),
            instructions: vec![
                MachineInstr::Store {
                    slot: StackSlot(0),
                    src: Reg::Rax,
                },
                MachineInstr::Load {
                    dst: Reg::R9,
                    slot: StackSlot(1),
                },
                MachineInstr::Add {
                    dst: Reg::Rax,
                    src: Operand::Stack(StackSlot(0)),
                },
                MachineInstr::Sub {
                    dst: Reg::R9,
                    src: Operand::Reg(Reg::R10),
                },
                MachineInstr::IMul {
                    dst: Reg::R12,
                    src: Operand::Stack(StackSlot(1)),
                },
                MachineInstr::Cmp {
                    lhs: Reg::Rax,
                    rhs: Operand::Stack(StackSlot(0)),
                },
                MachineInstr::SetCc {
                    condition: Condition::SignedGreater,
                    dst: Reg::R12,
                },
                MachineInstr::Ret,
            ],
            stack_slots: 2,
            callee_saved: vec![Reg::Rbx, Reg::R12],
            symbols: Vec::new(),
        };
        let code = function.emit_machine_code().unwrap();
        let expected = [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec, 0x20, 0x00, 0x00, 0x00, 0x48, 0x89, 0x9d,
            0xe8, 0xff, 0xff, 0xff, 0x4c, 0x89, 0xa5, 0xe0, 0xff, 0xff, 0xff, 0x48, 0x89, 0x85,
            0xf8, 0xff, 0xff, 0xff, 0x4c, 0x8b, 0x8d, 0xf0, 0xff, 0xff, 0xff, 0x48, 0x03, 0x85,
            0xf8, 0xff, 0xff, 0xff, 0x4d, 0x29, 0xd1, 0x4c, 0x0f, 0xaf, 0xa5, 0xf0, 0xff, 0xff,
            0xff, 0x48, 0x3b, 0x85, 0xf8, 0xff, 0xff, 0xff, 0x41, 0x0f, 0x9f, 0xc4, 0x45, 0x0f,
            0xb6, 0xe4, 0x48, 0x8b, 0x9d, 0xe8, 0xff, 0xff, 0xff, 0x4c, 0x8b, 0xa5, 0xe0, 0xff,
            0xff, 0xff, 0xc9, 0xc3,
        ];
        assert_eq!(code, expected);
    }

    #[test]
    fn signed_division_has_known_register_extended_and_stack_encodings() {
        let code = MachineFunction {
            instructions: vec![
                MachineInstr::Cqo,
                MachineInstr::IDiv {
                    src: Operand::Reg(Reg::Rcx),
                },
                MachineInstr::Cqo,
                MachineInstr::IDiv {
                    src: Operand::Reg(Reg::R8),
                },
                MachineInstr::Cqo,
                MachineInstr::IDiv {
                    src: Operand::Stack(StackSlot(0)),
                },
                MachineInstr::Ret,
            ],
            stack_slots: 1,
            ..function(Vec::new())
        }
        .emit_machine_code()
        .unwrap();
        assert_eq!(
            code,
            [
                0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec, 0x10, 0x00, 0x00, 0x00, 0x48, 0x99, 0x48,
                0xf7, 0xf9, 0x48, 0x99, 0x49, 0xf7, 0xf8, 0x48, 0x99, 0x48, 0xf7, 0xbd, 0xf8, 0xff,
                0xff, 0xff, 0xc9, 0xc3,
            ]
        );

        for divisor in [Reg::Rax, Reg::Rdx] {
            assert_eq!(
                function(vec![
                    MachineInstr::IDiv {
                        src: Operand::Reg(divisor),
                    },
                    MachineInstr::Ret,
                ])
                .emit_machine_code(),
                Err(EncodeError::InvalidDivisor(divisor))
            );
        }
    }

    #[test]
    fn all_conditions_and_byte_register_rex_prefixes_are_encoded() {
        let conditions = [
            (Condition::Eq, 0x94),
            (Condition::NotEq, 0x95),
            (Condition::SignedLess, 0x9c),
            (Condition::SignedLessEq, 0x9e),
            (Condition::SignedGreater, 0x9f),
            (Condition::SignedGreaterEq, 0x9d),
            (Condition::UnsignedBelow, 0x92),
            (Condition::UnsignedBelowEq, 0x96),
            (Condition::UnsignedAbove, 0x97),
            (Condition::UnsignedAboveEq, 0x93),
        ];
        for (condition, opcode) in conditions {
            let code = function(vec![
                MachineInstr::Cmp {
                    lhs: Reg::Rsi,
                    rhs: Operand::Reg(Reg::Rdi),
                },
                MachineInstr::SetCc {
                    condition,
                    dst: Reg::Rsi,
                },
                MachineInstr::SetCc {
                    condition,
                    dst: Reg::R15,
                },
                MachineInstr::Ret,
            ])
            .emit_machine_code()
            .unwrap();
            let mut expected = vec![
                0x55, 0x48, 0x89, 0xe5, 0x48, 0x39, 0xfe, 0x40, 0x0f, opcode, 0xc6,
            ];
            expected.extend_from_slice(&[0x40, 0x0f, 0xb6, 0xf6, 0x41, 0x0f, opcode, 0xc7]);
            expected.extend_from_slice(&[0x45, 0x0f, 0xb6, 0xff, 0xc9, 0xc3]);
            assert_eq!(code, expected, "condition {condition:?}");
        }
    }

    #[test]
    fn register_cmp_and_small_negative_immediate_have_known_encodings() {
        let code = function(vec![
            MachineInstr::Cmp {
                lhs: Reg::R11,
                rhs: Operand::Reg(Reg::Rbx),
            },
            MachineInstr::MovImm {
                dst: Reg::R15,
                value: -1,
            },
            MachineInstr::Ret,
        ])
        .emit_machine_code()
        .unwrap();
        assert_eq!(
            code,
            [
                0x55, 0x48, 0x89, 0xe5, 0x49, 0x39, 0xdb, 0x49, 0xc7, 0xc7, 0xff, 0xff, 0xff, 0xff,
                0xc9, 0xc3,
            ]
        );
    }

    #[test]
    fn rejects_unencodable_or_invalid_functions_before_emitting() {
        assert_eq!(
            function(Vec::new()).emit_machine_code(),
            Err(EncodeError::MissingReturn)
        );
        let reserved = function(vec![
            MachineInstr::Mov {
                dst: Reg::Rsp,
                src: Reg::Rax,
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(
            reserved.emit_machine_code(),
            Err(EncodeError::ReservedRegister(Reg::Rsp))
        );
        let bad_slot = function(vec![
            MachineInstr::Load {
                dst: Reg::Rax,
                slot: StackSlot(0),
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(
            bad_slot.emit_machine_code(),
            Err(EncodeError::StackSlotOutOfBounds {
                slot: StackSlot(0),
                stack_slots: 0,
            })
        );
        let caller_saved = MachineFunction {
            callee_saved: vec![Reg::Rax],
            ..function(vec![MachineInstr::Ret])
        };
        assert_eq!(
            caller_saved.emit_machine_code(),
            Err(EncodeError::ExpectedCalleeSaved(Reg::Rax))
        );
        let duplicate = MachineFunction {
            callee_saved: vec![Reg::Rbx, Reg::Rbx],
            ..function(vec![MachineInstr::Ret])
        };
        assert_eq!(
            duplicate.emit_machine_code(),
            Err(EncodeError::DuplicateCalleeSaved(Reg::Rbx))
        );
        let huge_frame = MachineFunction {
            stack_slots: u32::MAX,
            ..function(vec![MachineInstr::Ret])
        };
        assert_eq!(
            huge_frame.emit_machine_code(),
            Err(EncodeError::FrameSizeOverflow)
        );
        for index in [usize::MAX, i32::MAX as usize / 8 + Reg::ARGUMENTS.len()] {
            let argument = function(vec![
                MachineInstr::LoadArg {
                    dst: Reg::Rax,
                    index,
                },
                MachineInstr::Ret,
            ]);
            assert_eq!(
                argument.emit_machine_code(),
                Err(EncodeError::ArgumentOffsetOverflow { index })
            );
        }
    }

    #[test]
    fn system_objdump_accepts_the_encoded_elf_text_when_available() {
        let Some(objdump) = find_objdump() else {
            eprintln!("objdump/llvm-objdump not found; skipping external validation");
            return;
        };
        let function = MachineFunction {
            name: "disassemble_me".into(),
            instructions: vec![
                MachineInstr::LoadArg {
                    dst: Reg::Rax,
                    index: 0,
                },
                MachineInstr::MovImm {
                    dst: Reg::Rcx,
                    value: 3,
                },
                MachineInstr::IMul {
                    dst: Reg::Rax,
                    src: Operand::Reg(Reg::Rcx),
                },
                MachineInstr::Cqo,
                MachineInstr::IDiv {
                    src: Operand::Reg(Reg::Rcx),
                },
                MachineInstr::Store {
                    slot: StackSlot(0),
                    src: Reg::Rax,
                },
                MachineInstr::Cmp {
                    lhs: Reg::Rax,
                    rhs: Operand::Stack(StackSlot(0)),
                },
                MachineInstr::SetCc {
                    condition: Condition::Eq,
                    dst: Reg::Rsi,
                },
                MachineInstr::Ret,
            ],
            stack_slots: 1,
            callee_saved: Vec::new(),
            symbols: Vec::new(),
        };
        let code = function.emit_machine_code().unwrap();
        let object = write_object(
            &code,
            &[FunctionSymbol::new("disassemble_me", 0, code.len() as u64)],
        )
        .unwrap();
        let file = TempObject::new(&object);
        let output = Command::new(objdump)
            .args(["-d", "-M", "intel"])
            .arg(&file.0)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "{objdump} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        for expected in ["mov", "imul", "cqo", "idiv", "cmp", "sete", "leave", "ret"] {
            assert!(
                stdout.contains(expected),
                "{objdump} output lacks {expected:?}:\n{stdout}"
            );
        }
    }

    fn find_objdump() -> Option<&'static str> {
        ["objdump", "llvm-objdump"].into_iter().find(|command| {
            Command::new(command)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
    }

    struct TempObject(PathBuf);

    impl TempObject {
        fn new(contents: &[u8]) -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!("kfrc-x86-test-{}-{id}.o", process::id()));
            fs::write(&path, contents).unwrap();
            Self(path)
        }
    }

    impl Drop for TempObject {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
}
