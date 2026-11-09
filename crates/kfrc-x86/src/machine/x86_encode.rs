//! x86_64 machine-code emission for the native instruction IR.

use crate::machine::x86::{
    BitWidth, Condition, MachineFunction, MachineInstr, Operand, Reg, StackSlot, Xmm,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallRelocation {
    pub offset: u64,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressRelocation {
    pub offset: u64,
    pub target: String,
    pub addend: i64,
    pub kind: AddressRelocationKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressRelocationKind {
    PcRelative,
    TlsLocalExec,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncodeError {
    InvalidBitScanFallback,
    InvalidVectorRegister(u8),
    ReservedRegister(Reg),
    ExpectedCalleeSaved(Reg),
    DuplicateCalleeSaved(Reg),
    InvalidDivisor(Reg),
    InvalidMultiplyRegister(Reg),
    ImmediateDivision,
    InvalidCallTarget(u32),
    AddressRelocationsRequired,
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
            Self::InvalidBitScanFallback => {
                formatter.write_str("bit scan fallback aliases destination")
            }
            Self::InvalidVectorRegister(reg) => write!(formatter, "invalid XMM register {reg}"),
            Self::ReservedRegister(reg) => {
                write!(formatter, "{reg} is reserved for frame management")
            }
            Self::ExpectedCalleeSaved(reg) => {
                write!(formatter, "{reg} is not a callee-saved register")
            }
            Self::DuplicateCalleeSaved(reg) => {
                write!(formatter, "duplicate callee-saved register {reg}")
            }
            Self::InvalidMultiplyRegister(reg) => {
                write!(formatter, "{reg} is reserved by checked multiplication")
            }
            Self::InvalidDivisor(reg) => {
                write!(formatter, "{reg} cannot be used as an idiv divisor")
            }
            Self::ImmediateDivision => {
                formatter.write_str("x86 does not support immediate division")
            }
            Self::AddressRelocationsRequired => {
                formatter.write_str("symbol addresses require encode_with_all_relocations")
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
    let (bytes, calls, addresses) = encode_with_all_relocations(function)?;
    if !addresses.is_empty() {
        return Err(EncodeError::AddressRelocationsRequired);
    }
    Ok((bytes, calls))
}

pub fn encode_with_all_relocations(
    function: &MachineFunction,
) -> Result<(Vec<u8>, Vec<CallRelocation>, Vec<AddressRelocation>), EncodeError> {
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
        needs_frame: function.needs_frame(),
        callee_saved: &function.callee_saved,
        symbols: &function.symbols,
        calls: Vec::new(),
        addresses: Vec::new(),
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
    Ok((encoder.bytes, encoder.calls, encoder.addresses))
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

    for instruction in &function.instructions {
        validate_instruction(*instruction, function.stack_slots, &function.symbols)?;
    }
    let frame_size = function
        .frame_size()
        .ok_or(EncodeError::FrameSizeOverflow)?;
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
            Operand::Imm(_) => Ok(()),
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
        | Rotate { dst, .. }
        | ByteSwap { dst, .. }
        | SetCc { dst, .. } => validate_reg(dst)?,
        Extend { dst, src, .. } => {
            validate_reg(dst)?;
            validate_reg(src)?;
        }
        BitScanZero { dst, src, zero, .. } => {
            validate_reg(dst)?;
            validate_reg(src)?;
            validate_reg(zero)?;
            if dst == zero {
                return Err(EncodeError::InvalidBitScanFallback);
            }
        }
        Mov { dst, src } | OffsetAddress { dst, base: src, .. } => {
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
            if matches!(src, Operand::Imm(_)) {
                return Err(EncodeError::ImmediateDivision);
            }
            validate_operand(src, stack_slots)?;
            if let Operand::Reg(reg @ (Reg::Rax | Reg::Rdx)) = src {
                return Err(EncodeError::InvalidDivisor(reg));
            }
        }
        Cmp { lhs, rhs } => {
            validate_reg(lhs)?;
            validate_operand(rhs, stack_slots)?;
        }
        Test { reg } | TestByte { reg } => validate_reg(reg)?,
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
        StackAddress { dst, offset } => {
            validate_reg(dst)?;
            if offset >= 0 || i64::from(offset) < -(i64::from(stack_slots) * 8) {
                return Err(EncodeError::FrameSizeOverflow);
            }
        }
        LoadIndirect {
            dst,
            address,
            bytes,
            ..
        } => {
            validate_reg(dst)?;
            validate_reg(address)?;
            if !matches!(bytes, 1 | 2 | 4 | 8) {
                return Err(EncodeError::FrameSizeOverflow);
            }
        }
        StoreIndirect {
            address,
            src,
            bytes,
            ..
        } => {
            validate_reg(address)?;
            validate_reg(src)?;
            if !matches!(bytes, 1 | 2 | 4 | 8) {
                return Err(EncodeError::FrameSizeOverflow);
            }
        }
        StoreImmediate { address, bytes, .. } => {
            validate_reg(address)?;
            if !matches!(bytes, 1 | 2 | 4 | 8) {
                return Err(EncodeError::FrameSizeOverflow);
            }
        }
        LoadVector { dst, address } | StoreVector { address, src: dst } => {
            validate_reg(address)?;
            if dst.0 >= 16 {
                return Err(EncodeError::InvalidVectorRegister(dst.0));
            }
        }
        SymbolAddress { dst, symbol, .. } | ThreadLocalAddress { dst, symbol } => {
            validate_reg(dst)?;
            if symbol as usize >= symbols.len() {
                return Err(EncodeError::InvalidCallTarget(symbol));
            }
        }
        MulOverflow {
            dst,
            lhs,
            rhs,
            bits,
            ..
        } => {
            validate_reg(dst)?;
            validate_reg(lhs)?;
            validate_reg(rhs)?;
            if !matches!(bits, 8 | 16 | 32 | 64) {
                return Err(EncodeError::FrameSizeOverflow);
            }
            for register in [dst, lhs, rhs] {
                if matches!(register, Reg::Rax | Reg::Rdx) {
                    return Err(EncodeError::InvalidMultiplyRegister(register));
                }
            }
        }
        Label(_) | Jmp(_) | Jcc { .. } => {}
        CallIndirect { target } => validate_reg(target)?,
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

/// Encoder-private label of the shared epilogue; lowering numbers labels from zero.
const EPILOGUE_LABEL: u32 = u32::MAX;

struct JumpFixup {
    offset: usize,
    target: u32,
    short: bool,
}

struct Encoder<'a> {
    bytes: Vec<u8>,
    stack_slots: u32,
    needs_frame: bool,
    callee_saved: &'a [Reg],
    symbols: &'a [String],
    calls: Vec<CallRelocation>,
    addresses: Vec<AddressRelocation>,
    labels: BTreeMap<u32, usize>,
    fixups: Vec<JumpFixup>,
}

impl Encoder<'_> {
    fn prologue(&mut self, frame_size: u32) -> Result<(), EncodeError> {
        if !self.needs_frame {
            return Ok(());
        }
        self.bytes.extend_from_slice(&[0x55, 0x48, 0x89, 0xe5]);
        // Pushing after the local area stores register `index` exactly at
        // `saved_offset(stack_slots, index)`; the rest of the frame follows.
        let locals = self.stack_slots * 8;
        let saved = u32::try_from(self.callee_saved.len() * 8)
            .map_err(|_| EncodeError::FrameSizeOverflow)?;
        let rest = frame_size
            .checked_sub(locals + saved)
            .ok_or(EncodeError::FrameSizeOverflow)?;
        if self.callee_saved.is_empty() {
            self.adjust_rsp(frame_size);
            return Ok(());
        }
        self.adjust_rsp(locals);
        for reg in self.callee_saved.iter().copied() {
            self.rex(false, None, Some(reg), false);
            self.bytes.push(0x50 + reg.low_code());
        }
        self.adjust_rsp(rest);
        Ok(())
    }

    fn adjust_rsp(&mut self, bytes: u32) {
        if bytes == 0 {
            return;
        }
        self.bytes.push(0x48);
        if let Ok(short) = i8::try_from(bytes as i32) {
            self.bytes.extend_from_slice(&[0x83, 0xec, short as u8]);
        } else {
            self.bytes.extend_from_slice(&[0x81, 0xec]);
            self.i32(bytes as i32);
        }
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
                // A 32-bit move zero-extends, so it covers every value in
                // 0..=u32::MAX; only negative imm32 values need REX.W C7.
                if u32::try_from(value).is_ok() {
                    self.rex(false, None, Some(dst), false);
                    self.bytes.push(0xb8 + dst.low_code());
                    self.i32(value as i32);
                } else if i32::try_from(value).is_ok() {
                    self.rex(true, None, Some(dst), false);
                    self.bytes.push(0xc7);
                    self.modrm(3, Reg::Rax, dst.low_code());
                    self.i32(value as i32);
                } else {
                    self.rex(true, None, Some(dst), false);
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
            // The 32-bit zero idiom clears the whole register and needs no REX.W.
            Xor {
                dst,
                src: Operand::Reg(src),
            } if src == dst => {
                self.rex(false, Some(dst), Some(dst), false);
                self.bytes.push(0x31);
                self.modrm(3, dst, dst.low_code());
            }
            Xor { dst, src } => self.binary(0x31, 0x33, dst, src)?,
            Rotate {
                dst,
                width,
                right,
                count,
            } => {
                let bits = width.bits();
                if bits == 16 {
                    self.bytes.push(0x66);
                }
                self.rex(bits == 64, None, Some(dst), bits == 8 && dst.code() >= 4);
                self.bytes.push(match (bits == 8, count) {
                    (true, Some(_)) => 0xc0,
                    (false, Some(_)) => 0xc1,
                    (true, None) => 0xd2,
                    (false, None) => 0xd3,
                });
                self.modrm(3, if right { Reg::Rcx } else { Reg::Rax }, dst.low_code());
                if let Some(count) = count {
                    self.bytes.push(count);
                }
            }
            Extend {
                dst,
                src,
                width,
                signed,
            } => match (width, signed) {
                (BitWidth::W64, _) => self.reg_reg(0x89, dst, src),
                // A 32-bit move clears the upper half.
                (BitWidth::W32, false) => {
                    self.rex(false, Some(src), Some(dst), false);
                    self.bytes.push(0x89);
                    self.modrm(3, src, dst.low_code());
                }
                (BitWidth::W32, true) => {
                    self.rex(true, Some(dst), Some(src), false);
                    self.bytes.push(0x63);
                    self.modrm(3, dst, src.low_code());
                }
                (width, signed) => {
                    // spl/bpl/sil/dil need a REX prefix as byte sources.
                    let byte = width == BitWidth::W8 && (4..8).contains(&src.code());
                    self.rex(signed, Some(dst), Some(src), byte);
                    let opcode = match (width, signed) {
                        (BitWidth::W8, false) => 0xb6,
                        (_, false) => 0xb7,
                        (BitWidth::W8, true) => 0xbe,
                        _ => 0xbf,
                    };
                    self.bytes.extend_from_slice(&[0x0f, opcode]);
                    self.modrm(3, dst, src.low_code());
                }
            },
            ByteSwap { dst, wide } => {
                self.rex(wide, None, Some(dst), false);
                self.bytes.extend_from_slice(&[0x0f, 0xc8 + dst.low_code()]);
            }
            BitScanZero {
                dst,
                src,
                zero,
                reverse,
            } => {
                self.rex(true, Some(dst), Some(src), false);
                self.bytes
                    .extend_from_slice(&[0x0f, if reverse { 0xbd } else { 0xbc }]);
                self.modrm(3, dst, src.low_code());
                self.rex(true, Some(dst), Some(zero), false);
                self.bytes.extend_from_slice(&[0x0f, 0x44]);
                self.modrm(3, dst, zero.low_code());
            }
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
            ShlImm { dst, count } => self.shift_immediate(dst, Reg::Rsp, count),
            ShrImm { dst, count } => self.shift_immediate(dst, Reg::Rdi, count),
            LShrImm { dst, count } => self.shift_immediate(dst, Reg::Rbp, count),
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
                Operand::Imm(value) => {
                    self.rex(true, Some(dst), Some(dst), false);
                    if let Ok(value) = i8::try_from(value) {
                        self.bytes.push(0x6b);
                        self.modrm(3, dst, dst.low_code());
                        self.bytes.push(value as u8);
                    } else {
                        self.bytes.push(0x69);
                        self.modrm(3, dst, dst.low_code());
                        self.i32(value);
                    }
                }
            },
            Cqo => self.bytes.extend_from_slice(&[0x48, 0x99]),
            IDiv { src } => match src {
                Operand::Imm(_) => return Err(EncodeError::ImmediateDivision),
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
                Operand::Imm(_) => return Err(EncodeError::ImmediateDivision),
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
            Cmp {
                lhs,
                rhs: Operand::Imm(0),
            }
            | Test { reg: lhs } => self.reg_reg(0x85, lhs, lhs),
            TestByte { reg } => {
                // spl/bpl/sil/dil need a REX prefix as byte registers.
                self.rex(false, Some(reg), Some(reg), (4..8).contains(&reg.code()));
                self.bytes.push(0x84);
                self.modrm(3, reg, reg.low_code());
            }
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
            StackAddress { dst, offset } => self.memory_reg(0x8d, dst, offset),
            OffsetAddress { dst, base, offset } => {
                self.rex(true, Some(dst), Some(base), false);
                self.bytes.push(0x8d);
                self.base_memory(dst, base, offset);
            }
            LoadIndirect {
                dst,
                address,
                offset,
                bytes,
            } => self.indirect(true, dst, address, offset, bytes),
            StoreIndirect {
                address,
                offset,
                src,
                bytes,
            } => self.indirect(false, src, address, offset, bytes),
            StoreImmediate {
                address,
                offset,
                value,
                bytes,
            } => {
                if bytes == 2 {
                    self.bytes.push(0x66);
                }
                self.rex(bytes == 8, None, Some(address), false);
                self.bytes.push(if bytes == 1 { 0xc6 } else { 0xc7 });
                // ModRM.reg is the /0 opcode extension.
                self.base_memory(Reg::Rax, address, offset);
                match bytes {
                    1 => self.bytes.push(value as u8),
                    2 => self.bytes.extend_from_slice(&(value as i16).to_le_bytes()),
                    _ => self.i32(value),
                }
            }
            LoadVector { dst, address } => self.vector_memory(true, dst, address),
            StoreVector { address, src } => self.vector_memory(false, src, address),
            SymbolAddress {
                dst,
                symbol,
                addend,
            } => {
                self.rex(true, Some(dst), None, false);
                self.bytes.push(0x8d);
                self.modrm(0, dst, 5);
                self.addresses.push(AddressRelocation {
                    offset: self.bytes.len() as u64,
                    target: self.symbols[symbol as usize].clone(),
                    addend,
                    kind: AddressRelocationKind::PcRelative,
                });
                self.i32(0);
            }
            ThreadLocalAddress { dst, symbol } => {
                // mov dst, fs:[0], with an absolute SIB disp32 (not RIP-relative).
                self.bytes.push(0x64);
                self.rex(true, Some(dst), None, false);
                self.bytes.push(0x8b);
                self.modrm(0, dst, 4);
                self.bytes.push(0x25);
                self.i32(0);
                // lea dst, [dst + symbol@TPOFF]. LEA preserves arithmetic flags.
                self.rex(true, Some(dst), Some(dst), false);
                self.bytes.push(0x8d);
                self.modrm(2, dst, dst.low_code());
                if dst.low_code() == 4 {
                    self.bytes.push(0x24);
                }
                self.addresses.push(AddressRelocation {
                    offset: self.bytes.len() as u64,
                    target: self.symbols[symbol as usize].clone(),
                    addend: 0,
                    kind: AddressRelocationKind::TlsLocalExec,
                });
                self.i32(0);
            }
            MulOverflow {
                dst,
                lhs,
                rhs,
                signed,
                bits,
            } => {
                // Sources are preserved by the stack-machine lowerer in r10/r11.
                self.reg_reg(0x89, Reg::Rax, lhs);
                if signed {
                    self.rex(true, Some(Reg::Rax), Some(rhs), false);
                    self.bytes.extend_from_slice(&[0x0f, 0xaf]);
                    self.modrm(3, Reg::Rax, rhs.low_code());
                    // seto dst; movzx dst,dst8
                    self.rex(false, None, Some(dst), dst.code() >= 4);
                    self.bytes.extend_from_slice(&[0x0f, 0x90]);
                    self.modrm(3, Reg::Rax, dst.low_code());
                    self.rex(false, Some(dst), Some(dst), dst.code() >= 4);
                    self.bytes.extend_from_slice(&[0x0f, 0xb6]);
                    self.modrm(3, dst, dst.low_code());
                } else {
                    self.rex(true, None, Some(rhs), false);
                    self.bytes.push(0xf7);
                    self.modrm(3, Reg::Rsp, rhs.low_code());
                    self.reg_reg(0x85, Reg::Rdx, Reg::Rdx);
                    self.instruction(SetCc {
                        condition: Condition::NotEq,
                        dst,
                    })?;
                }
                if bits < 64 {
                    // Narrow inputs cannot overflow the 64-bit product; compare
                    // the mathematical product with its width-canonical value.
                    self.reg_reg(0x89, Reg::Rdx, Reg::Rax);
                    self.shift_immediate(Reg::Rdx, Reg::Rsp, 64 - bits);
                    self.shift_immediate(
                        Reg::Rdx,
                        if signed { Reg::Rdi } else { Reg::Rbp },
                        64 - bits,
                    );
                    self.reg_reg(0x39, Reg::Rax, Reg::Rdx);
                    self.instruction(SetCc {
                        condition: Condition::NotEq,
                        dst,
                    })?;
                }
            }
            CallIndirect { target } => {
                self.rex(true, None, Some(target), false);
                self.bytes.push(0xff);
                self.modrm(3, Reg::Rdx, target.low_code());
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
                let offset = self.bytes.len();
                self.bytes.extend_from_slice(&[0xe9, 0, 0, 0, 0]);
                self.fixups.push(JumpFixup {
                    offset,
                    target,
                    short: false,
                });
            }
            Jcc { condition, target } => {
                let offset = self.bytes.len();
                self.bytes
                    .extend_from_slice(&[0x0f, 0x80 + condition_code(condition)]);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                self.fixups.push(JumpFixup {
                    offset,
                    target,
                    short: false,
                });
            }
            Ret => {
                // Restoring callee-saved registers costs at least six bytes, so
                // later returns jump to the first return's shared epilogue.
                if !self.callee_saved.is_empty() {
                    if self.labels.contains_key(&EPILOGUE_LABEL) {
                        return self.instruction(Jmp(EPILOGUE_LABEL));
                    }
                    self.labels.insert(EPILOGUE_LABEL, self.bytes.len());
                }
                if let Some(last) = self.callee_saved.len().checked_sub(1) {
                    // lea rsp, [rbp + offset of the last pushed register]
                    let offset = saved_offset(self.stack_slots, last)?;
                    self.bytes.push(0x48);
                    self.bytes.push(0x8d);
                    self.stack_memory(Reg::Rsp, offset);
                    for reg in self.callee_saved.iter().rev().copied() {
                        self.rex(false, None, Some(reg), false);
                        self.bytes.push(0x58 + reg.low_code());
                    }
                }
                if self.needs_frame {
                    self.bytes.push(0xc9);
                }
                self.bytes.push(0xc3);
            }
        }
        Ok(())
    }

    fn resolve_labels(&mut self) -> Result<(), EncodeError> {
        loop {
            let mut removed_prefix = Vec::with_capacity(self.fixups.len() + 1);
            let mut removed = 0;
            removed_prefix.push(0);
            for fixup in &self.fixups {
                if fixup.short {
                    removed += if self.bytes[fixup.offset] == 0xe9 {
                        3
                    } else {
                        4
                    };
                }
                removed_prefix.push(removed);
            }

            let mut shortened = false;
            for index in 0..self.fixups.len() {
                if self.fixups[index].short {
                    continue;
                }
                let original_offset = self.fixups[index].offset;
                let target = self.fixups[index].target;
                let offset = original_offset - removed_prefix[index];
                let destination = self
                    .labels
                    .get(&target)
                    .copied()
                    .ok_or(EncodeError::UnknownLabel(target))?;
                let destination = layout_offset(destination, &self.fixups, &removed_prefix);
                let unconditional = self.bytes[original_offset] == 0xe9;
                let long_size = if unconditional { 5 } else { 6 };
                let short_displacement = if destination > offset {
                    destination as i128 - (offset as i128 + long_size)
                } else {
                    destination as i128 - (offset as i128 + 2)
                };
                if i8::try_from(short_displacement).is_ok() {
                    self.fixups[index].short = true;
                    shortened = true;
                }
            }
            if !shortened {
                self.apply_shortened_layout(&removed_prefix);
                break;
            }
        }

        for fixup in &self.fixups {
            let destination = self
                .labels
                .get(&fixup.target)
                .copied()
                .ok_or(EncodeError::UnknownLabel(fixup.target))?;
            if fixup.short {
                let displacement = destination as i128 - (fixup.offset as i128 + 2);
                let displacement = i8::try_from(displacement)
                    .map_err(|_| EncodeError::RelocationOverflow(fixup.target))?;
                self.bytes[fixup.offset + 1] = displacement as u8;
            } else {
                let displacement_offset = if self.bytes[fixup.offset] == 0xe9 {
                    fixup.offset + 1
                } else {
                    fixup.offset + 2
                };
                let displacement = destination as i128 - (displacement_offset as i128 + 4);
                let displacement = i32::try_from(displacement)
                    .map_err(|_| EncodeError::RelocationOverflow(fixup.target))?;
                self.bytes[displacement_offset..displacement_offset + 4]
                    .copy_from_slice(&displacement.to_le_bytes());
            }
        }
        Ok(())
    }

    fn apply_shortened_layout(&mut self, removed_prefix: &[usize]) {
        let mut read_offset = 0;
        let mut write_offset = 0;
        for fixup in self.fixups.iter().filter(|fixup| fixup.short) {
            let offset = fixup.offset;
            self.bytes.copy_within(read_offset..offset, write_offset);
            write_offset += offset - read_offset;
            if self.bytes[offset] == 0xe9 {
                self.bytes[write_offset] = 0xeb;
                self.bytes[write_offset + 1] = self.bytes[offset + 2];
                read_offset = offset + 5;
            } else {
                self.bytes[write_offset] = 0x70 + (self.bytes[offset + 1] & 0x0f);
                self.bytes[write_offset + 1] = self.bytes[offset + 2];
                read_offset = offset + 6;
            }
            write_offset += 2;
        }
        let original_len = self.bytes.len();
        self.bytes
            .copy_within(read_offset..original_len, write_offset);
        write_offset += original_len - read_offset;
        self.bytes.truncate(write_offset);

        for label_offset in self.labels.values_mut() {
            *label_offset = layout_offset(*label_offset, &self.fixups, removed_prefix);
        }
        for call in &mut self.calls {
            call.offset = layout_offset(call.offset as usize, &self.fixups, removed_prefix) as u64;
        }
        for address in &mut self.addresses {
            address.offset =
                layout_offset(address.offset as usize, &self.fixups, removed_prefix) as u64;
        }
        for (index, fixup) in self.fixups.iter_mut().enumerate() {
            fixup.offset -= removed_prefix[index];
        }
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
            Operand::Imm(value) => self.group_immediate(reg_opcode, dst, value),
        }
        Ok(())
    }

    fn group_immediate(&mut self, opcode: u8, dst: Reg, value: i32) {
        let extension = match opcode {
            0x01 => Reg::Rax,
            0x09 => Reg::Rcx,
            0x21 => Reg::Rsp,
            0x29 => Reg::Rbp,
            0x31 => Reg::Rsi,
            0x39 => Reg::Rdi,
            _ => unreachable!("not a group-one arithmetic opcode"),
        };
        self.rex(true, None, Some(dst), false);
        if let Ok(value) = i8::try_from(value) {
            self.bytes.push(0x83);
            self.modrm(3, extension, dst.low_code());
            self.bytes.push(value as u8);
        } else {
            self.bytes.push(0x81);
            self.modrm(3, extension, dst.low_code());
            self.i32(value);
        }
    }

    fn shift_immediate(&mut self, dst: Reg, extension: Reg, count: u8) {
        self.rex(true, None, Some(dst), false);
        self.bytes.push(if count == 1 { 0xd1 } else { 0xc1 });
        self.modrm(3, extension, dst.low_code());
        if count != 1 {
            self.bytes.push(count);
        }
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

    fn vector_memory(&mut self, load: bool, reg: Xmm, address: Reg) {
        self.bytes.push(0xf3); // MOVDQU permits unaligned addressable values.
        let rex = 0x40 | if reg.0 >= 8 { 4 } else { 0 } | if address.code() >= 8 { 1 } else { 0 };
        if rex != 0x40 {
            self.bytes.push(rex);
        }
        self.bytes
            .extend_from_slice(&[0x0f, if load { 0x6f } else { 0x7f }]);
        self.bytes
            .push(0x40 | ((reg.0 & 7) << 3) | address.low_code());
        if address.low_code() == 4 {
            self.bytes.push(0x24);
        }
        self.bytes.push(0); // disp8=0 also handles RBP/R13.
    }

    fn indirect(&mut self, load: bool, reg: Reg, address: Reg, offset: i32, bytes: u8) {
        if !load && bytes == 2 {
            self.bytes.push(0x66);
        }
        self.rex(
            bytes == 8 || (load && bytes < 4),
            Some(reg),
            Some(address),
            !load && bytes == 1 && reg.code() >= 4,
        );
        if load && bytes < 4 {
            self.bytes
                .extend_from_slice(&[0x0f, if bytes == 1 { 0xb6 } else { 0xb7 }]);
        } else {
            self.bytes.push(if load {
                0x8b
            } else if bytes == 1 {
                0x88
            } else {
                0x89
            });
        }
        self.base_memory(reg, address, offset);
    }

    /// ModRM, SIB and displacement for `[address + offset]`.
    fn base_memory(&mut self, reg: Reg, address: Reg, offset: i32) {
        // mod=00 with rm=101 means RIP-relative, so rbp/r13 bases keep disp8=0.
        let short = i8::try_from(offset).ok();
        let mode = match short {
            Some(0) if address.low_code() != 5 => 0,
            Some(_) => 1,
            None => 2,
        };
        self.modrm(mode, reg, address.low_code());
        if address.low_code() == 4 {
            self.bytes.push(0x24);
        }
        match mode {
            1 => self.bytes.push(offset as u8),
            2 => self.i32(offset),
            _ => {}
        }
    }

    fn stack_memory(&mut self, reg: Reg, offset: i32) {
        if let Ok(offset) = i8::try_from(offset) {
            self.modrm(1, reg, Reg::Rbp.low_code());
            self.bytes.push(offset as u8);
        } else {
            self.modrm(2, reg, Reg::Rbp.low_code());
            self.i32(offset);
        }
    }

    fn stack_memory_rsp(&mut self, reg: Reg, offset: i32) {
        if offset == 0 {
            self.modrm(0, reg, Reg::Rsp.low_code());
            self.bytes.push(0x24);
        } else if let Ok(offset) = i8::try_from(offset) {
            self.modrm(1, reg, Reg::Rsp.low_code());
            self.bytes.extend_from_slice(&[0x24, offset as u8]);
        } else {
            self.modrm(2, reg, Reg::Rsp.low_code());
            self.bytes.push(0x24);
            self.i32(offset);
        }
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

fn layout_offset(offset: usize, fixups: &[JumpFixup], removed_prefix: &[usize]) -> usize {
    let index = fixups.partition_point(|fixup| fixup.offset + 2 < offset);
    offset - removed_prefix[index]
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
    #[test]
    fn vector_memory_rejects_invalid_xmm_registers_and_frame_register_addresses() {
        use crate::machine::x86::{MachineFunction, MachineInstr, Reg, Xmm};
        for instruction in [
            MachineInstr::LoadVector {
                dst: Xmm(16),
                address: Reg::Rax,
            },
            MachineInstr::StoreVector {
                address: Reg::Rax,
                src: Xmm(255),
            },
            MachineInstr::LoadVector {
                dst: Xmm(0),
                address: Reg::Rsp,
            },
            MachineInstr::StoreVector {
                address: Reg::Rbp,
                src: Xmm(0),
            },
        ] {
            let mut function = MachineFunction::new("invalid_vector");
            function.instructions = vec![instruction, MachineInstr::Ret];
            assert!(super::encode_with_all_relocations(&function).is_err());
            assert!(function.emit_gnu_assembly().is_err());
        }
    }

    #[test]
    fn local_exec_tls_keeps_segment_rex_sib_and_relocations_after_branch_shortening() {
        use super::{AddressRelocation, AddressRelocationKind, EncodeError};
        use crate::machine::x86::{MachineFunction, MachineInstr, Reg};
        let mut function = MachineFunction::new("tls_address");
        function.symbols = vec!["tls_symbol".into()];
        function.instructions = vec![
            MachineInstr::Jmp(1),
            MachineInstr::ThreadLocalAddress {
                dst: Reg::R12,
                symbol: 0,
            },
            MachineInstr::Label(1),
            MachineInstr::Ret,
        ];
        let (bytes, calls, addresses) = super::encode_with_all_relocations(&function).unwrap();
        assert_eq!(
            bytes,
            [
                0xeb, 17, 0x64, 0x4c, 0x8b, 0x24, 0x25, 0, 0, 0, 0, 0x4d, 0x8d, 0xa4, 0x24, 0, 0,
                0, 0, 0xc3,
            ]
        );
        assert!(calls.is_empty());
        assert_eq!(
            addresses,
            [AddressRelocation {
                offset: 15,
                target: "tls_symbol".into(),
                addend: 0,
                kind: AddressRelocationKind::TlsLocalExec,
            }]
        );
        assert_eq!(
            super::encode_with_relocations(&function),
            Err(EncodeError::AddressRelocationsRequired)
        );
        for (dst, symbol) in [(Reg::Rsp, 0), (Reg::Rbp, 0), (Reg::Rax, 1)] {
            function.instructions = vec![
                MachineInstr::ThreadLocalAddress { dst, symbol },
                MachineInstr::Ret,
            ];
            assert!(super::encode_with_all_relocations(&function).is_err());
            assert!(function.emit_gnu_assembly().is_err());
        }
    }

    #[test]
    fn pointer_memory_widths_and_extended_registers_encode_correctly() {
        let mut function = crate::machine::x86::MachineFunction::new("memory_widths");
        function.instructions = vec![
            crate::machine::x86::MachineInstr::LoadIndirect {
                dst: crate::machine::x86::Reg::Rax,
                address: crate::machine::x86::Reg::R12,
                offset: 0,
                bytes: 1,
            },
            crate::machine::x86::MachineInstr::StoreIndirect {
                address: crate::machine::x86::Reg::R13,
                offset: 0,
                src: crate::machine::x86::Reg::Rsi,
                bytes: 1,
            },
            crate::machine::x86::MachineInstr::LoadIndirect {
                dst: crate::machine::x86::Reg::R11,
                address: crate::machine::x86::Reg::R10,
                offset: 0,
                bytes: 4,
            },
            crate::machine::x86::MachineInstr::StoreIndirect {
                address: crate::machine::x86::Reg::R10,
                offset: 0,
                src: crate::machine::x86::Reg::R11,
                bytes: 2,
            },
            crate::machine::x86::MachineInstr::Ret,
        ];
        let bytes = super::emit_machine_code(&function).unwrap();
        assert_eq!(
            &bytes[..bytes.len() - 1],
            &[
                0x49, 0x0f, 0xb6, 0x04, 0x24, // movzx rax, byte [r12]
                0x41, 0x88, 0x75, 0x00, // mov byte [r13 + 0], sil (no mod=00 form)
                0x45, 0x8b, 0x1a, // mov r11d, dword [r10]
                0x66, 0x45, 0x89, 0x1a, // mov word [r10], r11w
            ]
        );
    }

    #[test]
    fn symbol_address_relocation_moves_with_shortened_branches() {
        let mut function = crate::machine::x86::MachineFunction::new("symbol_address");
        function.symbols = vec!["data_symbol".to_owned()];
        function.instructions = vec![
            crate::machine::x86::MachineInstr::Jmp(1),
            crate::machine::x86::MachineInstr::SymbolAddress {
                dst: crate::machine::x86::Reg::Rax,
                symbol: 0,
                addend: 17,
            },
            crate::machine::x86::MachineInstr::Label(1),
            crate::machine::x86::MachineInstr::Ret,
        ];
        let (bytes, calls, addresses) = super::encode_with_all_relocations(&function).unwrap();
        assert_eq!(bytes[..2], [0xeb, 7]);
        assert!(calls.is_empty());
        assert_eq!(
            addresses,
            vec![super::AddressRelocation {
                offset: 5,
                target: "data_symbol".to_owned(),
                addend: 17,
                kind: super::AddressRelocationKind::PcRelative,
            }]
        );
        assert_eq!(&bytes[2..5], &[0x48, 0x8d, 0x05]);
    }

    use super::*;
    use crate::{FunctionCode, SymbolBinding, write_object};
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
    fn empty_leaf_omits_frame_but_calls_and_stack_arguments_keep_it() {
        assert_eq!(
            function(vec![MachineInstr::Ret])
                .emit_machine_code()
                .unwrap(),
            [0xc3]
        );
        for instruction in [
            MachineInstr::Call { target: 0 },
            MachineInstr::CallIndirect { target: Reg::Rax },
            MachineInstr::LoadArg {
                dst: Reg::Rax,
                index: 6,
            },
        ] {
            let mut function = function(vec![instruction, MachineInstr::Ret]);
            function.symbols.push("callee".into());
            let (bytes, _, _) = encode_with_all_relocations(&function).unwrap();
            assert!(bytes.starts_with(&[0x55, 0x48, 0x89, 0xe5]));
            assert!(bytes.ends_with(&[0xc9, 0xc3]));
            let assembly = function.emit_gnu_assembly().unwrap();
            assert!(assembly.contains("push rbp\n\tmov rbp, rsp"));
            assert!(assembly.contains("leave\n\tret"));
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
            0x48, 0x89, 0xf9, 0x48, 0x01, 0xc8, 0x4c, 0x8b, 0x45, 0x10, 0x4c, 0x29, 0xc0, 0x48,
            0x0f, 0xaf, 0xc1, 0x49, 0x89, 0xc1, 0xc9, 0xc3,
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
            // push rbp; mov rbp, rsp; sub rsp, 16; push rbx; push r12
            0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 0x10, 0x53, 0x41, 0x54, //
            0x48, 0x89, 0x45, 0xf8, 0x4c, 0x8b, 0x4d, 0xf0, 0x48, 0x03, 0x45, 0xf8, 0x4d, 0x29,
            0xd1, 0x4c, 0x0f, 0xaf, 0x65, 0xf0, 0x48, 0x3b, 0x45, 0xf8, 0x41, 0x0f, 0x9f, 0xc4,
            0x45, 0x0f, 0xb6, 0xe4,
            // lea rsp, [rbp - 0x20]; pop r12; pop rbx; leave; ret
            0x48, 0x8d, 0x65, 0xe0, 0x41, 0x5c, 0x5b, 0xc9, 0xc3,
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
                0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 0x10, 0x48, 0x99, 0x48, 0xf7, 0xf9, 0x48,
                0x99, 0x49, 0xf7, 0xf8, 0x48, 0x99, 0x48, 0xf7, 0x7d, 0xf8, 0xc9, 0xc3,
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
            let mut expected = vec![0x48, 0x39, 0xfe, 0x40, 0x0f, opcode, 0xc6];
            expected.extend_from_slice(&[0x40, 0x0f, 0xb6, 0xf6, 0x41, 0x0f, opcode, 0xc7]);
            expected.extend_from_slice(&[0x45, 0x0f, 0xb6, 0xff, 0xc3]);
            assert_eq!(code, expected, "condition {condition:?}");
        }
    }

    #[test]
    fn byte_tests_use_rex_only_for_new_byte_registers() {
        for (reg, expected) in [
            (Reg::Rax, vec![0x84, 0xc0]),
            (Reg::Rbx, vec![0x84, 0xdb]),
            (Reg::Rsi, vec![0x40, 0x84, 0xf6]),
            (Reg::Rdi, vec![0x40, 0x84, 0xff]),
            (Reg::R8, vec![0x45, 0x84, 0xc0]),
            (Reg::R15, vec![0x45, 0x84, 0xff]),
        ] {
            let code = function(vec![MachineInstr::TestByte { reg }, MachineInstr::Ret])
                .emit_machine_code()
                .unwrap();
            let mut expected = expected;
            expected.push(0xc3);
            assert_eq!(code, expected, "{reg:?}");
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
                0x49, 0x39, 0xdb, 0x49, 0xc7, 0xc7, 0xff, 0xff, 0xff, 0xff, 0xc3,
            ]
        );
    }

    #[test]
    fn rbp_and_rsp_stack_displacements_have_exact_boundary_encodings() {
        let cases: &[(i32, &[u8], &[u8])] = &[
            (
                -129,
                &[0x85, 0x7f, 0xff, 0xff, 0xff],
                &[0x84, 0x24, 0x7f, 0xff, 0xff, 0xff],
            ),
            (-128, &[0x45, 0x80], &[0x44, 0x24, 0x80]),
            (0, &[0x45, 0x00], &[0x04, 0x24]),
            (127, &[0x45, 0x7f], &[0x44, 0x24, 0x7f]),
            (
                128,
                &[0x85, 0x80, 0x00, 0x00, 0x00],
                &[0x84, 0x24, 0x80, 0x00, 0x00, 0x00],
            ),
            (
                129,
                &[0x85, 0x81, 0x00, 0x00, 0x00],
                &[0x84, 0x24, 0x81, 0x00, 0x00, 0x00],
            ),
        ];
        let mut encoder = Encoder {
            bytes: Vec::new(),
            stack_slots: 0,
            needs_frame: true,
            callee_saved: &[],
            symbols: &[],
            calls: Vec::new(),
            addresses: Vec::new(),
            labels: BTreeMap::new(),
            fixups: Vec::new(),
        };
        for (offset, expected_rbp, expected_rsp) in cases {
            encoder.bytes.clear();
            encoder.stack_memory(Reg::Rax, *offset);
            assert_eq!(encoder.bytes.as_slice(), *expected_rbp, "rbp{offset:+}");

            encoder.bytes.clear();
            encoder.stack_memory_rsp(Reg::Rax, *offset);
            assert_eq!(encoder.bytes.as_slice(), *expected_rsp, "rsp{offset:+}");
        }
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
        let object = write_object(&[FunctionCode {
            function,
            binding: SymbolBinding::Global { hidden: false },
        }])
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
