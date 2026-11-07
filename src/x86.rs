//! Minimal x86_64 System V code generation without external dependencies.
//!
//! Virtual instructions are three-address and retain their lowering order. Machine
//! instructions use x86's two-address form: for example, `Add { dst, src }` means
//! `dst += src`. Register allocation is responsible for inserting `Load` and
//! `Store` instructions for spill slots.

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VirtualReg(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StackSlot(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualValue {
    Reg(VirtualReg),
    Imm(i64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Condition {
    Eq,
    NotEq,
    SignedLess,
    SignedLessEq,
    SignedGreater,
    SignedGreaterEq,
    UnsignedBelow,
    UnsignedBelowEq,
    UnsignedAbove,
    UnsignedAboveEq,
}

impl Condition {
    fn mnemonic(self) -> &'static str {
        match self {
            Self::Eq => "e",
            Self::NotEq => "ne",
            Self::SignedLess => "l",
            Self::SignedLessEq => "le",
            Self::SignedGreater => "g",
            Self::SignedGreaterEq => "ge",
            Self::UnsignedBelow => "b",
            Self::UnsignedBelowEq => "be",
            Self::UnsignedAbove => "a",
            Self::UnsignedAboveEq => "ae",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VirtualInstr {
    LoadArg {
        dst: VirtualReg,
        index: usize,
    },
    Mov {
        dst: VirtualReg,
        src: VirtualValue,
    },
    Neg {
        dst: VirtualReg,
        src: VirtualValue,
    },
    Not {
        dst: VirtualReg,
        src: VirtualValue,
    },
    Add {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Sub {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Mul {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    And {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Or {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Xor {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Shl {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Shr {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    LShr {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Div {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Rem {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    UDiv {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    URem {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Compare {
        dst: VirtualReg,
        condition: Condition,
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Test {
        lhs: VirtualValue,
        rhs: VirtualValue,
    },
    Load {
        dst: VirtualReg,
        slot: StackSlot,
    },
    Store {
        slot: StackSlot,
        src: VirtualValue,
    },
    Label(u32),
    Jmp(u32),
    Jcc {
        condition: Condition,
        target: u32,
    },
    Phi {
        dst: VirtualReg,
    },
    Call {
        dst: VirtualReg,
        target: String,
        args: Vec<VirtualValue>,
    },
    Return(VirtualValue),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Reg {
    Rax,
    Rbx,
    Rcx,
    Rdx,
    Rsi,
    Rdi,
    Rbp,
    Rsp,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

impl Reg {
    pub const ARGUMENTS: [Self; 6] = [
        Self::Rdi,
        Self::Rsi,
        Self::Rdx,
        Self::Rcx,
        Self::R8,
        Self::R9,
    ];
    pub const RETURN: Self = Self::Rax;
    pub const CALLER_SAVED: [Self; 9] = [
        Self::Rax,
        Self::Rcx,
        Self::Rdx,
        Self::Rsi,
        Self::Rdi,
        Self::R8,
        Self::R9,
        Self::R10,
        Self::R11,
    ];
    pub const CALLEE_SAVED: [Self; 6] = [
        Self::Rbx,
        Self::Rbp,
        Self::R12,
        Self::R13,
        Self::R14,
        Self::R15,
    ];
    pub const ALLOCATABLE: [Self; 14] = [
        Self::Rax,
        Self::Rcx,
        Self::Rdx,
        Self::Rsi,
        Self::Rdi,
        Self::R8,
        Self::R9,
        Self::R10,
        Self::R11,
        Self::Rbx,
        Self::R12,
        Self::R13,
        Self::R14,
        Self::R15,
    ];

    pub const fn is_caller_saved(self) -> bool {
        matches!(
            self,
            Self::Rax
                | Self::Rcx
                | Self::Rdx
                | Self::Rsi
                | Self::Rdi
                | Self::R8
                | Self::R9
                | Self::R10
                | Self::R11
        )
    }

    pub const fn is_callee_saved(self) -> bool {
        matches!(
            self,
            Self::Rbx | Self::Rbp | Self::R12 | Self::R13 | Self::R14 | Self::R15
        )
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Rax => "rax",
            Self::Rbx => "rbx",
            Self::Rcx => "rcx",
            Self::Rdx => "rdx",
            Self::Rsi => "rsi",
            Self::Rdi => "rdi",
            Self::Rbp => "rbp",
            Self::Rsp => "rsp",
            Self::R8 => "r8",
            Self::R9 => "r9",
            Self::R10 => "r10",
            Self::R11 => "r11",
            Self::R12 => "r12",
            Self::R13 => "r13",
            Self::R14 => "r14",
            Self::R15 => "r15",
        }
    }

    const fn low8(self) -> &'static str {
        match self {
            Self::Rax => "al",
            Self::Rbx => "bl",
            Self::Rcx => "cl",
            Self::Rdx => "dl",
            Self::Rsi => "sil",
            Self::Rdi => "dil",
            Self::Rbp => "bpl",
            Self::Rsp => "spl",
            Self::R8 => "r8b",
            Self::R9 => "r9b",
            Self::R10 => "r10b",
            Self::R11 => "r11b",
            Self::R12 => "r12b",
            Self::R13 => "r13b",
            Self::R14 => "r14b",
            Self::R15 => "r15b",
        }
    }

    const fn low32(self) -> &'static str {
        match self {
            Self::Rax => "eax",
            Self::Rbx => "ebx",
            Self::Rcx => "ecx",
            Self::Rdx => "edx",
            Self::Rsi => "esi",
            Self::Rdi => "edi",
            Self::Rbp => "ebp",
            Self::Rsp => "esp",
            Self::R8 => "r8d",
            Self::R9 => "r9d",
            Self::R10 => "r10d",
            Self::R11 => "r11d",
            Self::R12 => "r12d",
            Self::R13 => "r13d",
            Self::R14 => "r14d",
            Self::R15 => "r15d",
        }
    }

    const fn is_reserved(self) -> bool {
        matches!(self, Self::Rbp | Self::Rsp)
    }

    const fn is_savable(self) -> bool {
        matches!(
            self,
            Self::Rbx | Self::R12 | Self::R13 | Self::R14 | Self::R15
        )
    }
}

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operand {
    Reg(Reg),
    Stack(StackSlot),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MachineInstr {
    LoadArg { dst: Reg, index: usize },
    MovImm { dst: Reg, value: i64 },
    Mov { dst: Reg, src: Reg },
    Neg { dst: Reg },
    Not { dst: Reg },
    Add { dst: Reg, src: Operand },
    Sub { dst: Reg, src: Operand },
    IMul { dst: Reg, src: Operand },
    And { dst: Reg, src: Operand },
    Or { dst: Reg, src: Operand },
    Xor { dst: Reg, src: Operand },
    Shl { dst: Reg },
    Shr { dst: Reg },
    LShr { dst: Reg },
    Cqo,
    IDiv { src: Operand },
    UDiv { src: Operand },
    Cmp { lhs: Reg, rhs: Operand },
    SetCc { condition: Condition, dst: Reg },
    Load { dst: Reg, slot: StackSlot },
    Store { slot: StackSlot, src: Reg },
    StoreArg { src: Reg, index: usize },
    Label(u32),
    Jmp(u32),
    Jcc { condition: Condition, target: u32 },
    Call { target: u32 },
    Ret,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineFunction {
    pub name: String,
    pub instructions: Vec<MachineInstr>,
    pub stack_slots: u32,
    pub callee_saved: Vec<Reg>,
    pub symbols: Vec<String>,
}

impl MachineFunction {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            instructions: Vec::new(),
            stack_slots: 0,
            callee_saved: Vec::new(),
            symbols: Vec::new(),
        }
    }

    pub fn emit_gnu_assembly(&self) -> Result<String, AsmError> {
        emit_gnu_assembly(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AsmError {
    InvalidFunctionName,
    ReservedRegister(Reg),
    ExpectedCalleeSaved(Reg),
    DuplicateCalleeSaved(Reg),
    InvalidDivisor(Reg),
    InvalidCallTarget(u32),
    StackSlotOutOfBounds { slot: StackSlot, stack_slots: u32 },
    ArgumentOffsetOverflow { index: usize },
    FrameSizeOverflow,
    MissingReturn,
}

impl fmt::Display for AsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFunctionName => f.write_str("invalid GNU assembly function name"),
            Self::ReservedRegister(reg) => write!(f, "{reg} is reserved for frame management"),
            Self::ExpectedCalleeSaved(reg) => write!(f, "{reg} is not a callee-saved register"),
            Self::DuplicateCalleeSaved(reg) => write!(f, "duplicate callee-saved register {reg}"),
            Self::InvalidDivisor(reg) => write!(f, "{reg} cannot be used as an idiv divisor"),
            Self::InvalidCallTarget(target) => {
                write!(f, "call target symbol index {target} is invalid")
            }
            Self::StackSlotOutOfBounds { slot, stack_slots } => write!(
                f,
                "stack slot {} is outside the allocated {} slots",
                slot.0, stack_slots
            ),
            Self::ArgumentOffsetOverflow { index } => {
                write!(f, "stack argument offset overflows for argument {index}")
            }
            Self::FrameSizeOverflow => f.write_str("stack frame size overflow"),
            Self::MissingReturn => f.write_str("machine function has no return instruction"),
        }
    }
}

impl std::error::Error for AsmError {}

pub fn emit_gnu_assembly(function: &MachineFunction) -> Result<String, AsmError> {
    validate_function(function)?;

    let saved_bytes = function
        .callee_saved
        .len()
        .checked_mul(8)
        .ok_or(AsmError::FrameSizeOverflow)?;
    let spill_bytes = (function.stack_slots as usize)
        .checked_mul(8)
        .ok_or(AsmError::FrameSizeOverflow)?;
    let frame_bytes = spill_bytes
        .checked_add(saved_bytes)
        .ok_or(AsmError::FrameSizeOverflow)?
        .checked_add(15)
        .ok_or(AsmError::FrameSizeOverflow)?
        & !15;

    let mut out = String::new();
    out.push_str(".intel_syntax noprefix\n.text\n.globl ");
    out.push_str(&function.name);
    out.push_str("\n.type ");
    out.push_str(&function.name);
    out.push_str(", @function\n");
    out.push_str(&function.name);
    out.push_str(":\n\tpush rbp\n\tmov rbp, rsp\n");
    if frame_bytes != 0 {
        out.push_str(&format!("\tsub rsp, {frame_bytes}\n"));
    }
    for (index, reg) in function.callee_saved.iter().enumerate() {
        let offset = saved_offset(function.stack_slots, index)?;
        out.push_str(&format!("\tmov QWORD PTR [rbp{offset}], {}\n", reg.name()));
    }

    for instruction in &function.instructions {
        emit_instruction(
            &mut out,
            *instruction,
            function.stack_slots,
            &function.callee_saved,
            &function.symbols,
        )?;
    }

    out.push_str(".size ");
    out.push_str(&function.name);
    out.push_str(", .-");
    out.push_str(&function.name);
    out.push_str("\n.section .note.GNU-stack,\"\",@progbits\n");
    Ok(out)
}

fn validate_function(function: &MachineFunction) -> Result<(), AsmError> {
    if !valid_symbol(&function.name) {
        return Err(AsmError::InvalidFunctionName);
    }
    for (index, reg) in function.callee_saved.iter().copied().enumerate() {
        if reg.is_reserved() {
            return Err(AsmError::ReservedRegister(reg));
        }
        if !reg.is_savable() {
            return Err(AsmError::ExpectedCalleeSaved(reg));
        }
        if function.callee_saved[..index].contains(&reg) {
            return Err(AsmError::DuplicateCalleeSaved(reg));
        }
    }
    if !function.instructions.contains(&MachineInstr::Ret) {
        return Err(AsmError::MissingReturn);
    }
    for instruction in &function.instructions {
        validate_instruction(*instruction, function.stack_slots, &function.symbols)?;
    }
    Ok(())
}

fn valid_symbol(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || matches!(first, b'_' | b'.' | b'$'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$'))
}

fn validate_instruction(
    instruction: MachineInstr,
    stack_slots: u32,
    symbols: &[String],
) -> Result<(), AsmError> {
    use MachineInstr::*;

    let validate_reg = |reg: Reg| {
        if reg.is_reserved() {
            Err(AsmError::ReservedRegister(reg))
        } else {
            Ok(())
        }
    };
    let validate_operand = |operand: Operand| match operand {
        Operand::Reg(reg) => validate_reg(reg),
        Operand::Stack(slot) => validate_slot(slot, stack_slots),
    };

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
            validate_operand(src)?;
        }
        Cqo => {}
        IDiv { src } | UDiv { src } => {
            validate_operand(src)?;
            if let Operand::Reg(reg @ (Reg::Rax | Reg::Rdx)) = src {
                return Err(AsmError::InvalidDivisor(reg));
            }
        }
        Cmp { lhs, rhs } => {
            validate_reg(lhs)?;
            validate_operand(rhs)?;
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
                return Err(AsmError::ArgumentOffsetOverflow { index });
            }
            incoming_arg_offset(index)?;
        }
        Label(_) | Jmp(_) | Jcc { .. } => {}
        Call { target } => {
            if target as usize >= symbols.len() {
                return Err(AsmError::InvalidCallTarget(target));
            }
        }
        Ret => {}
    }
    Ok(())
}

fn validate_slot(slot: StackSlot, stack_slots: u32) -> Result<(), AsmError> {
    if slot.0 < stack_slots {
        Ok(())
    } else {
        Err(AsmError::StackSlotOutOfBounds { slot, stack_slots })
    }
}

fn emit_instruction(
    out: &mut String,
    instruction: MachineInstr,
    stack_slots: u32,
    callee_saved: &[Reg],
    symbols: &[String],
) -> Result<(), AsmError> {
    use MachineInstr::*;

    match instruction {
        LoadArg { dst, index } => {
            if let Some(src) = Reg::ARGUMENTS.get(index) {
                out.push_str(&format!("\tmov {}, {}\n", dst.name(), src.name()));
            } else {
                let offset = incoming_arg_offset(index)?;
                out.push_str(&format!("\tmov {}, QWORD PTR [rbp+{offset}]\n", dst.name()));
            }
        }
        MovImm { dst, value } => {
            if i32::try_from(value).is_ok() {
                out.push_str(&format!("\tmov {}, {}\n", dst.name(), value));
            } else {
                out.push_str(&format!("\tmovabs {}, 0x{:x}\n", dst.name(), value as u64));
            }
        }
        Mov { dst, src } => {
            out.push_str(&format!("\tmov {}, {}\n", dst.name(), src.name()));
        }
        Neg { dst } => out.push_str(&format!("\tneg {}\n", dst.name())),
        Not { dst } => out.push_str(&format!("\tnot {}\n", dst.name())),
        Add { dst, src } => emit_binary(out, "add", dst, src, stack_slots)?,
        Sub { dst, src } => emit_binary(out, "sub", dst, src, stack_slots)?,
        IMul { dst, src } => emit_binary(out, "imul", dst, src, stack_slots)?,
        And { dst, src } => emit_binary(out, "and", dst, src, stack_slots)?,
        Or { dst, src } => emit_binary(out, "or", dst, src, stack_slots)?,
        Xor { dst, src } => emit_binary(out, "xor", dst, src, stack_slots)?,
        Shl { dst } => out.push_str(&format!("\tshl {}, cl\n", dst.name())),
        Shr { dst } => out.push_str(&format!("\tsar {}, cl\n", dst.name())),
        LShr { dst } => out.push_str(&format!("\tshr {}, cl\n", dst.name())),
        Cqo => out.push_str("\tcqo\n"),
        IDiv { src } => {
            out.push_str("\tidiv ");
            emit_operand(out, src, stack_slots)?;
            out.push('\n');
        }
        UDiv { src } => {
            out.push_str("\tdiv ");
            emit_operand(out, src, stack_slots)?;
            out.push('\n');
        }
        Cmp { lhs, rhs } => {
            out.push_str(&format!("\tcmp {}, ", lhs.name()));
            emit_operand(out, rhs, stack_slots)?;
            out.push('\n');
        }
        SetCc { condition, dst } => {
            out.push_str(&format!("\tset{} {}\n", condition.mnemonic(), dst.low8()));
            out.push_str(&format!("\tmovzx {}, {}\n", dst.low32(), dst.low8()));
        }
        Load { dst, slot } => {
            let offset = spill_offset(slot);
            out.push_str(&format!("\tmov {}, QWORD PTR [rbp{offset}]\n", dst.name()));
        }
        Store { slot, src } => {
            let offset = spill_offset(slot);
            out.push_str(&format!("\tmov QWORD PTR [rbp{offset}], {}\n", src.name()));
        }
        StoreArg { src, index } => {
            let offset = (index - Reg::ARGUMENTS.len()) * 8;
            out.push_str(&format!("\tmov QWORD PTR [rsp+{offset}], {}\n", src.name()));
        }
        Label(label) => out.push_str(&format!(".L{label}:\n")),
        Jmp(target) => out.push_str(&format!("\tjmp .L{target}\n")),
        Jcc { condition, target } => {
            out.push_str(&format!("\tj{} .L{target}\n", condition.mnemonic()))
        }
        Call { target } => out.push_str(&format!("\tcall {}\n", symbols[target as usize])),
        Ret => {
            for (index, reg) in callee_saved.iter().enumerate() {
                let offset = saved_offset(stack_slots, index)?;
                out.push_str(&format!("\tmov {}, QWORD PTR [rbp{offset}]\n", reg.name()));
            }
            out.push_str("\tleave\n\tret\n");
        }
    }
    Ok(())
}

fn emit_binary(
    out: &mut String,
    mnemonic: &str,
    dst: Reg,
    src: Operand,
    stack_slots: u32,
) -> Result<(), AsmError> {
    out.push_str(&format!("\t{mnemonic} {}, ", dst.name()));
    emit_operand(out, src, stack_slots)?;
    out.push('\n');
    Ok(())
}

fn emit_operand(out: &mut String, operand: Operand, stack_slots: u32) -> Result<(), AsmError> {
    match operand {
        Operand::Reg(reg) => out.push_str(reg.name()),
        Operand::Stack(slot) => {
            validate_slot(slot, stack_slots)?;
            out.push_str(&format!("QWORD PTR [rbp{}]", spill_offset(slot)));
        }
    }
    Ok(())
}

fn spill_offset(slot: StackSlot) -> String {
    format!("-{}", (slot.0 as u64 + 1) * 8)
}

fn saved_offset(stack_slots: u32, index: usize) -> Result<String, AsmError> {
    let slot = (stack_slots as usize)
        .checked_add(index)
        .and_then(|value| value.checked_add(1))
        .and_then(|value| value.checked_mul(8))
        .ok_or(AsmError::FrameSizeOverflow)?;
    Ok(format!("-{slot}"))
}

fn incoming_arg_offset(index: usize) -> Result<usize, AsmError> {
    index
        .checked_sub(Reg::ARGUMENTS.len())
        .and_then(|stack_index| stack_index.checked_mul(8))
        .and_then(|offset| offset.checked_add(16))
        .ok_or(AsmError::ArgumentOffsetOverflow { index })
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

    #[test]
    fn system_v_register_classes_match_the_abi() {
        assert_eq!(
            Reg::ARGUMENTS,
            [Reg::Rdi, Reg::Rsi, Reg::Rdx, Reg::Rcx, Reg::R8, Reg::R9]
        );
        assert_eq!(Reg::RETURN, Reg::Rax);
        assert!(Reg::CALLER_SAVED.iter().all(|reg| reg.is_caller_saved()));
        assert!(Reg::CALLEE_SAVED.iter().all(|reg| reg.is_callee_saved()));
        assert!(!Reg::Rsp.is_caller_saved());
        assert!(!Reg::Rsp.is_callee_saved());
        assert!(!Reg::ALLOCATABLE.contains(&Reg::Rbp));
        assert!(!Reg::ALLOCATABLE.contains(&Reg::Rsp));
    }

    #[test]
    fn emits_register_and_stack_arguments_arithmetic_and_return() {
        let asm = function(vec![
            MachineInstr::LoadArg {
                dst: Reg::Rax,
                index: 0,
            },
            MachineInstr::LoadArg {
                dst: Reg::Rcx,
                index: 1,
            },
            MachineInstr::Add {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::LoadArg {
                dst: Reg::Rcx,
                index: 2,
            },
            MachineInstr::Sub {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::LoadArg {
                dst: Reg::Rcx,
                index: 3,
            },
            MachineInstr::IMul {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::LoadArg {
                dst: Reg::Rcx,
                index: 6,
            },
            MachineInstr::Add {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::Ret,
        ])
        .emit_gnu_assembly()
        .unwrap();

        let body = [
            "\tmov rax, rdi\n",
            "\tmov rcx, rsi\n",
            "\tadd rax, rcx\n",
            "\tmov rcx, rdx\n",
            "\tsub rax, rcx\n",
            "\tmov rcx, rcx\n",
            "\timul rax, rcx\n",
            "\tmov rcx, QWORD PTR [rbp+16]\n",
            "\tadd rax, rcx\n",
            "\tleave\n\tret\n",
        ]
        .concat();
        assert!(asm.contains(&body), "{asm}");
        assert!(asm.starts_with(".intel_syntax noprefix\n.text\n.globl test_fn\n"));
        assert!(
            asm.ends_with(".size test_fn, .-test_fn\n.section .note.GNU-stack,\"\",@progbits\n")
        );
    }

    #[test]
    fn comparisons_produce_zero_extended_i64_booleans() {
        let conditions = [
            (Condition::Eq, "e"),
            (Condition::NotEq, "ne"),
            (Condition::SignedLess, "l"),
            (Condition::SignedLessEq, "le"),
            (Condition::SignedGreater, "g"),
            (Condition::SignedGreaterEq, "ge"),
            (Condition::UnsignedBelow, "b"),
            (Condition::UnsignedBelowEq, "be"),
            (Condition::UnsignedAbove, "a"),
            (Condition::UnsignedAboveEq, "ae"),
        ];
        for (condition, mnemonic) in conditions {
            let asm = function(vec![
                MachineInstr::Cmp {
                    lhs: Reg::Rax,
                    rhs: Operand::Reg(Reg::Rcx),
                },
                MachineInstr::SetCc {
                    condition,
                    dst: Reg::R12,
                },
                MachineInstr::Mov {
                    dst: Reg::Rax,
                    src: Reg::R12,
                },
                MachineInstr::Ret,
            ]);
            let asm = MachineFunction {
                callee_saved: vec![Reg::R12],
                ..asm
            }
            .emit_gnu_assembly()
            .unwrap();
            assert!(asm.contains(&format!(
                "\tcmp rax, rcx\n\tset{mnemonic} r12b\n\tmovzx r12d, r12b\n"
            )));
        }
    }

    #[test]
    fn spill_slots_and_callee_saved_registers_use_distinct_frame_locations() {
        let asm = MachineFunction {
            name: "spills".into(),
            instructions: vec![
                MachineInstr::Store {
                    slot: StackSlot(0),
                    src: Reg::Rbx,
                },
                MachineInstr::Load {
                    dst: Reg::Rax,
                    slot: StackSlot(0),
                },
                MachineInstr::Add {
                    dst: Reg::Rax,
                    src: Operand::Stack(StackSlot(0)),
                },
                MachineInstr::Ret,
            ],
            stack_slots: 1,
            callee_saved: vec![Reg::Rbx, Reg::R12],
            symbols: Vec::new(),
        }
        .emit_gnu_assembly()
        .unwrap();

        let expected = "\tpush rbp\n\tmov rbp, rsp\n\tsub rsp, 32\n\
                        \tmov QWORD PTR [rbp-16], rbx\n\
                        \tmov QWORD PTR [rbp-24], r12\n\
                        \tmov QWORD PTR [rbp-8], rbx\n\
                        \tmov rax, QWORD PTR [rbp-8]\n\
                        \tadd rax, QWORD PTR [rbp-8]\n\
                        \tmov rbx, QWORD PTR [rbp-16]\n\
                        \tmov r12, QWORD PTR [rbp-24]\n\
                        \tleave\n\tret\n";
        assert!(asm.contains(expected), "{asm}");
        assert!(!asm.contains("push rbx"));
        assert!(!asm.contains("push r12"));
    }

    #[test]
    fn emits_signed_division_and_rejects_fixed_registers_as_divisors() {
        let asm = MachineFunction {
            instructions: vec![
                MachineInstr::Cqo,
                MachineInstr::IDiv {
                    src: Operand::Reg(Reg::Rcx),
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
        .emit_gnu_assembly()
        .unwrap();
        assert!(asm.contains("\tcqo\n\tidiv rcx\n\tcqo\n\tidiv QWORD PTR [rbp-8]\n"));

        for divisor in [Reg::Rax, Reg::Rdx] {
            assert_eq!(
                function(vec![
                    MachineInstr::IDiv {
                        src: Operand::Reg(divisor),
                    },
                    MachineInstr::Ret,
                ])
                .emit_gnu_assembly(),
                Err(AsmError::InvalidDivisor(divisor))
            );
        }
    }

    #[test]
    fn immediate_constants_keep_all_64_bits() {
        let asm = function(vec![
            MachineInstr::MovImm {
                dst: Reg::Rax,
                value: i64::MIN,
            },
            MachineInstr::Ret,
        ])
        .emit_gnu_assembly()
        .unwrap();
        assert!(asm.contains("\tmovabs rax, 0x8000000000000000\n"));
    }

    #[test]
    fn emission_does_not_reorder_machine_instructions() {
        let asm = function(vec![
            MachineInstr::Sub {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::Add {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rdx),
            },
            MachineInstr::IMul {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::Ret,
        ])
        .emit_gnu_assembly()
        .unwrap();
        let sub = asm.find("\tsub rax, rcx").unwrap();
        let add = asm.find("\tadd rax, rdx").unwrap();
        let mul = asm.find("\timul rax, rcx").unwrap();
        assert!(sub < add && add < mul);
    }

    #[test]
    fn virtual_instruction_api_represents_native_operations() {
        let v0 = VirtualReg(0);
        let v1 = VirtualReg(1);
        let instructions = [
            VirtualInstr::LoadArg { dst: v0, index: 0 },
            VirtualInstr::Mov {
                dst: v1,
                src: VirtualValue::Imm(2),
            },
            VirtualInstr::Mul {
                dst: v0,
                lhs: VirtualValue::Reg(v0),
                rhs: VirtualValue::Reg(v1),
            },
            VirtualInstr::Store {
                slot: StackSlot(0),
                src: VirtualValue::Reg(v0),
            },
            VirtualInstr::Load {
                dst: v1,
                slot: StackSlot(0),
            },
            VirtualInstr::Compare {
                dst: v0,
                condition: Condition::SignedGreater,
                lhs: VirtualValue::Reg(v1),
                rhs: VirtualValue::Imm(0),
            },
            VirtualInstr::Return(VirtualValue::Reg(v0)),
        ];
        assert_eq!(instructions.len(), 7);
    }

    #[test]
    fn rejects_invalid_frame_and_register_metadata() {
        let bad_slot = function(vec![
            MachineInstr::Load {
                dst: Reg::Rax,
                slot: StackSlot(0),
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(
            bad_slot.emit_gnu_assembly(),
            Err(AsmError::StackSlotOutOfBounds {
                slot: StackSlot(0),
                stack_slots: 0
            })
        );

        let caller_saved = MachineFunction {
            callee_saved: vec![Reg::Rax],
            ..function(vec![MachineInstr::Ret])
        };
        assert_eq!(
            caller_saved.emit_gnu_assembly(),
            Err(AsmError::ExpectedCalleeSaved(Reg::Rax))
        );

        let reserved = function(vec![
            MachineInstr::Mov {
                dst: Reg::Rsp,
                src: Reg::Rax,
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(
            reserved.emit_gnu_assembly(),
            Err(AsmError::ReservedRegister(Reg::Rsp))
        );
        assert_eq!(
            function(Vec::new()).emit_gnu_assembly(),
            Err(AsmError::MissingReturn)
        );
    }
}
