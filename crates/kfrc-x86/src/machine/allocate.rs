//! Allocation and scalar machine emission for already lowered instructions.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use super::regalloc::{
    AllocationResult, LiveInterval, Location, MoveHint, PhysicalRegister, RegAllocError,
    RegisterAllocator, SpillSlot, VirtualReg as AllocVirtualReg,
};
use super::x86::{
    MachineFunction, MachineInstr, Operand, Reg, StackSlot, VirtualInstr, VirtualReg, VirtualValue,
};

const SCRATCH0: Reg = Reg::R10;
const SCRATCH1: Reg = Reg::R11;
const CALL_HOME_REGISTERS: [Reg; 5] = [Reg::Rcx, Reg::Rsi, Reg::Rdi, Reg::R8, Reg::R9];
const CALL_HOME_COUNT: u32 = CALL_HOME_REGISTERS.len() as u32;
const GENERAL_REGISTERS: [Reg; 10] = [
    Reg::Rcx,
    Reg::Rsi,
    Reg::Rdi,
    Reg::R8,
    Reg::R9,
    Reg::Rbx,
    Reg::R12,
    Reg::R13,
    Reg::R14,
    Reg::R15,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompileError {
    TooManyInstructions,
    TooManyStackSlots,
    MissingAllocation(VirtualReg),
    RegisterAllocation(RegAllocError),
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyInstructions => {
                formatter.write_str("lowered function exceeds the instruction position space")
            }
            Self::TooManyStackSlots => formatter.write_str("function exceeds the stack slot space"),
            Self::MissingAllocation(virtual_reg) => write!(
                formatter,
                "register allocator returned no location for virtual register {}",
                virtual_reg.0
            ),
            Self::RegisterAllocation(error) => {
                write!(formatter, "register allocation failed: {error}")
            }
        }
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RegisterAllocation(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RegAllocError> for CompileError {
    fn from(error: RegAllocError) -> Self {
        Self::RegisterAllocation(error)
    }
}

/// Register-allocates and emits already lowered virtual instructions.
/// The caller supplies live intervals computed from its control-flow graph.
pub fn compile_virtual_function(
    virtual_instructions: Vec<VirtualInstr>,
    intervals: Vec<LiveInterval>,
    name: impl Into<String>,
) -> Result<MachineFunction, CompileError> {
    let mut intervals = intervals;
    // A variable shift count is loaded into `rcx`, which is also allocatable:
    // pin `rcx` at each such shift so no value lives there across it.
    let mut next_pin = virtual_instructions
        .iter()
        .flat_map(|instruction| {
            defined_register(instruction)
                .into_iter()
                .chain(used_registers(instruction))
        })
        .map(|register| register.0)
        .chain(intervals.iter().map(|interval| interval.virtual_reg.0))
        .max()
        .map_or(0, |max| max + 1);
    for (position, instruction) in virtual_instructions.iter().enumerate() {
        if let VirtualInstr::Shl {
            rhs: VirtualValue::Reg(_),
            ..
        }
        | VirtualInstr::Shr {
            rhs: VirtualValue::Reg(_),
            ..
        }
        | VirtualInstr::LShr {
            rhs: VirtualValue::Reg(_),
            ..
        } = instruction
        {
            let position =
                u32::try_from(position).map_err(|_| CompileError::TooManyInstructions)?;
            intervals.push(LiveInterval::fixed(
                AllocVirtualReg(next_pin),
                position,
                position + 1,
                physical_register(Reg::Rcx),
            ));
            next_pin += 1;
        }
    }
    let move_hints = collect_move_hints(&virtual_instructions)?;
    let allocator = RegisterAllocator::new(GENERAL_REGISTERS.len() as u16);
    let allocation = allocator.allocate_with_move_hints(&intervals, &move_hints)?;
    let argument_homes = required_argument_homes(&virtual_instructions, &allocation);
    let spill_slot_offset = argument_homes.len() as u32;
    // Rbp-relative caller saves and argument staging, not the Rsp-relative
    // outgoing argument area reserved independently by MachineFunction::frame_size.
    let call_slots = virtual_instructions
        .iter()
        .filter_map(|instruction| match instruction {
            VirtualInstr::Call { args, .. } => Some(args.len()),
            _ => None,
        })
        .max()
        .map(|count| count as u32 + CALL_HOME_COUNT)
        .unwrap_or(0);
    let stack_slots = allocation
        .spill_slot_count
        .checked_add(spill_slot_offset)
        .and_then(|slots| slots.checked_add(call_slots))
        .ok_or(CompileError::TooManyStackSlots)?;

    let mut function = MachineFunction::new(name);
    function.stack_slots = stack_slots;
    function.callee_saved = GENERAL_REGISTERS
        .iter()
        .copied()
        .filter(|register| {
            register.is_callee_saved()
                && allocation.iter().any(|(_, location)| {
                    location == Location::Register(physical_register(*register))
                })
        })
        .collect();

    for (index, slot) in &argument_homes {
        function.instructions.push(MachineInstr::Store {
            slot: StackSlot(*slot),
            src: Reg::ARGUMENTS[*index],
        });
    }
    function.symbols = virtual_instructions
        .iter()
        .filter_map(|instruction| match instruction {
            VirtualInstr::Call { target, .. } => Some(target.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut emitter = Emitter {
        allocation: &allocation,
        argument_homes: &argument_homes,
        spill_slot_offset,
        call_slot_offset: spill_slot_offset + allocation.spill_slot_count,
        symbols: &function.symbols,
        instructions: Vec::with_capacity(virtual_instructions.len()),
    };
    for instruction in &virtual_instructions {
        emitter.emit(instruction.clone())?;
    }
    function.instructions.extend(emitter.instructions);
    crate::machine::x86::optimize_zero_idioms(&mut function.instructions);
    Ok(function)
}

fn alloc_virtual_reg(virtual_reg: VirtualReg) -> AllocVirtualReg {
    AllocVirtualReg(virtual_reg.0)
}

fn physical_register(register: Reg) -> PhysicalRegister {
    let index = GENERAL_REGISTERS
        .iter()
        .position(|candidate| *candidate == register)
        .expect("register is in the general allocation set");
    PhysicalRegister(index as u16)
}

fn general_register(register: PhysicalRegister) -> Reg {
    GENERAL_REGISTERS[register.index()]
}

fn collect_move_hints(instructions: &[VirtualInstr]) -> Result<Vec<MoveHint>, CompileError> {
    let mut hints = Vec::new();
    for (index, instruction) in instructions.iter().enumerate() {
        let VirtualInstr::Mov {
            dst,
            src: VirtualValue::Reg(src),
        } = *instruction
        else {
            continue;
        };
        let position = u32::try_from(index).map_err(|_| CompileError::TooManyInstructions)?;
        hints.push(MoveHint::new(
            position,
            AllocVirtualReg(dst.0),
            AllocVirtualReg(src.0),
        ));
    }
    Ok(hints)
}

fn defined_register(instruction: &VirtualInstr) -> Option<VirtualReg> {
    match *instruction {
        VirtualInstr::LoadArg { dst, .. }
        | VirtualInstr::Mov { dst, .. }
        | VirtualInstr::Neg { dst, .. }
        | VirtualInstr::Not { dst, .. }
        | VirtualInstr::Add { dst, .. }
        | VirtualInstr::Sub { dst, .. }
        | VirtualInstr::Mul { dst, .. }
        | VirtualInstr::And { dst, .. }
        | VirtualInstr::Or { dst, .. }
        | VirtualInstr::Xor { dst, .. }
        | VirtualInstr::Shl { dst, .. }
        | VirtualInstr::Shr { dst, .. }
        | VirtualInstr::LShr { dst, .. }
        | VirtualInstr::Div { dst, .. }
        | VirtualInstr::Rem { dst, .. }
        | VirtualInstr::UDiv { dst, .. }
        | VirtualInstr::URem { dst, .. }
        | VirtualInstr::Compare { dst, .. }
        | VirtualInstr::Load { dst, .. }
        | VirtualInstr::Phi { dst, .. }
        | VirtualInstr::Call { dst, .. } => Some(dst),
        VirtualInstr::Test { .. }
        | VirtualInstr::Nop
        | VirtualInstr::Label(_)
        | VirtualInstr::Jmp(_)
        | VirtualInstr::Jcc { .. }
        | VirtualInstr::Store { .. }
        | VirtualInstr::Return(_) => None,
    }
}

fn used_registers(instruction: &VirtualInstr) -> Vec<VirtualReg> {
    let mut used = Vec::new();
    let mut add_value = |value: &VirtualValue| {
        if let VirtualValue::Reg(virtual_reg) = *value {
            used.push(virtual_reg);
        }
    };
    match instruction {
        VirtualInstr::Mov { src, .. }
        | VirtualInstr::Neg { src, .. }
        | VirtualInstr::Not { src, .. }
        | VirtualInstr::Return(src) => add_value(src),
        VirtualInstr::Add { lhs, rhs, .. }
        | VirtualInstr::Sub { lhs, rhs, .. }
        | VirtualInstr::Mul { lhs, rhs, .. }
        | VirtualInstr::And { lhs, rhs, .. }
        | VirtualInstr::Or { lhs, rhs, .. }
        | VirtualInstr::Xor { lhs, rhs, .. }
        | VirtualInstr::Shl { lhs, rhs, .. }
        | VirtualInstr::Shr { lhs, rhs, .. }
        | VirtualInstr::LShr { lhs, rhs, .. }
        | VirtualInstr::Div { lhs, rhs, .. }
        | VirtualInstr::Rem { lhs, rhs, .. }
        | VirtualInstr::UDiv { lhs, rhs, .. }
        | VirtualInstr::URem { lhs, rhs, .. }
        | VirtualInstr::Compare { lhs, rhs, .. }
        | VirtualInstr::Test { lhs, rhs } => {
            add_value(lhs);
            add_value(rhs);
        }
        VirtualInstr::Store { src, .. } => add_value(src),
        VirtualInstr::Call { args, .. } => {
            for value in args {
                add_value(value);
            }
        }
        VirtualInstr::Nop
        | VirtualInstr::Label(_)
        | VirtualInstr::Jmp(_)
        | VirtualInstr::Jcc { .. }
        | VirtualInstr::Phi { .. }
        | VirtualInstr::LoadArg { .. }
        | VirtualInstr::Load { .. } => {}
    }
    used
}

fn required_argument_homes(
    instructions: &[VirtualInstr],
    allocation: &AllocationResult,
) -> BTreeMap<usize, u32> {
    let mut clobbered = [false; Reg::ARGUMENTS.len()];
    let mut required = BTreeSet::new();

    for instruction in instructions {
        if let VirtualInstr::LoadArg { index, .. } = *instruction {
            if index < Reg::ARGUMENTS.len() && clobbered[index] {
                required.insert(index);
            }
        }
        if matches!(
            instruction,
            VirtualInstr::Div { .. }
                | VirtualInstr::Rem { .. }
                | VirtualInstr::UDiv { .. }
                | VirtualInstr::URem { .. }
        ) {
            let index = Reg::ARGUMENTS
                .iter()
                .position(|argument| *argument == Reg::Rdx)
                .expect("rdx is an argument register");
            clobbered[index] = true;
        }
        let Some(definition) = defined_register(instruction) else {
            continue;
        };
        let Some(Location::Register(register)) =
            allocation.allocation(alloc_virtual_reg(definition))
        else {
            continue;
        };
        let register = general_register(register);
        if let Some(index) = Reg::ARGUMENTS
            .iter()
            .position(|argument| *argument == register)
        {
            clobbered[index] = true;
        }
    }

    required
        .into_iter()
        .enumerate()
        .map(|(slot, index)| (index, slot as u32))
        .collect()
}

struct Emitter<'a> {
    allocation: &'a AllocationResult,
    argument_homes: &'a BTreeMap<usize, u32>,
    spill_slot_offset: u32,
    /// First slot of the call area (caller-saved homes, then staged
    /// arguments), after the argument homes and all spill slots.
    call_slot_offset: u32,
    symbols: &'a [String],
    instructions: Vec<MachineInstr>,
}

impl Emitter<'_> {
    fn emit(&mut self, instruction: VirtualInstr) -> Result<(), CompileError> {
        match instruction {
            VirtualInstr::LoadArg { dst, index } => {
                let work = self.definition_register(dst)?;
                if let Some(slot) = self.argument_homes.get(&index) {
                    self.instructions.push(MachineInstr::Load {
                        dst: work,
                        slot: StackSlot(*slot),
                    });
                } else {
                    self.instructions
                        .push(MachineInstr::LoadArg { dst: work, index });
                }
                self.store_definition(dst, work)?;
            }
            VirtualInstr::Mov { dst, src } => {
                let work = self.definition_register(dst)?;
                self.load_value(src, work)?;
                self.store_definition(dst, work)?;
            }
            VirtualInstr::Neg { dst, src } => {
                let work = self.definition_register(dst)?;
                self.load_value(src, work)?;
                self.instructions.push(MachineInstr::Neg { dst: work });
                self.store_definition(dst, work)?;
            }
            VirtualInstr::Not { dst, src } => {
                let work = self.definition_register(dst)?;
                self.load_value(src, work)?;
                self.instructions.push(MachineInstr::Not { dst: work });
                self.store_definition(dst, work)?;
            }
            VirtualInstr::Add { dst, lhs, rhs } => {
                self.emit_binary(dst, lhs, rhs, BinaryKind::Add)?;
            }
            VirtualInstr::Sub { dst, lhs, rhs } => {
                self.emit_binary(dst, lhs, rhs, BinaryKind::Sub)?;
            }
            VirtualInstr::Mul { dst, lhs, rhs } => {
                self.emit_binary(dst, lhs, rhs, BinaryKind::Mul)?;
            }
            VirtualInstr::And { dst, lhs, rhs } => {
                self.emit_binary(dst, lhs, rhs, BinaryKind::And)?;
            }
            VirtualInstr::Or { dst, lhs, rhs } => {
                self.emit_binary(dst, lhs, rhs, BinaryKind::Or)?;
            }
            VirtualInstr::Xor { dst, lhs, rhs } => {
                self.emit_binary(dst, lhs, rhs, BinaryKind::Xor)?;
            }
            VirtualInstr::Shl { dst, lhs, rhs } => {
                self.emit_shift(dst, lhs, rhs, ShiftKind::Left)?;
            }
            VirtualInstr::Shr { dst, lhs, rhs } => {
                self.emit_shift(dst, lhs, rhs, ShiftKind::Right)?;
            }
            VirtualInstr::LShr { dst, lhs, rhs } => {
                self.emit_shift(dst, lhs, rhs, ShiftKind::Logical)?;
            }
            VirtualInstr::Div { dst, lhs, rhs } => {
                if let VirtualValue::Imm(divisor) = rhs
                    && divisor > 0
                    && (divisor as u64).is_power_of_two()
                {
                    self.emit_signed_division_power_of_two(
                        dst,
                        lhs,
                        divisor.trailing_zeros() as u8,
                    )?;
                } else {
                    self.emit_division(dst, lhs, rhs, DivisionResult::Quotient, true)?;
                }
            }
            VirtualInstr::Rem { dst, lhs, rhs } => {
                self.emit_division(dst, lhs, rhs, DivisionResult::Remainder, true)?;
            }
            VirtualInstr::UDiv { dst, lhs, rhs } => {
                self.emit_division(dst, lhs, rhs, DivisionResult::Quotient, false)?;
            }
            VirtualInstr::URem { dst, lhs, rhs } => {
                self.emit_division(dst, lhs, rhs, DivisionResult::Remainder, false)?;
            }
            VirtualInstr::Compare {
                dst,
                condition,
                lhs,
                rhs,
            } => {
                let work = self.definition_register(dst)?;
                self.load_value(lhs, work)?;
                if rhs == VirtualValue::Imm(0) {
                    self.instructions.push(MachineInstr::Test { reg: work });
                } else {
                    let rhs = self.operand(rhs, SCRATCH1)?;
                    self.instructions.push(MachineInstr::Cmp { lhs: work, rhs });
                }
                self.instructions.push(MachineInstr::SetCc {
                    condition,
                    dst: work,
                });
                self.store_definition(dst, work)?;
            }
            VirtualInstr::Test { lhs, rhs } => {
                self.load_value(lhs, SCRATCH0)?;
                if rhs == VirtualValue::Imm(0) {
                    self.instructions.push(MachineInstr::Test { reg: SCRATCH0 });
                } else {
                    let rhs = self.operand(rhs, SCRATCH1)?;
                    self.instructions
                        .push(MachineInstr::Cmp { lhs: SCRATCH0, rhs });
                }
            }
            VirtualInstr::Nop => {}
            VirtualInstr::Label(label) => {
                self.instructions.push(MachineInstr::Label(label));
            }
            VirtualInstr::Jmp(target) => {
                self.instructions.push(MachineInstr::Jmp(target));
            }
            VirtualInstr::Jcc { condition, target } => {
                self.instructions
                    .push(MachineInstr::Jcc { condition, target });
            }
            VirtualInstr::Phi { .. } => {}
            VirtualInstr::Call { dst, target, args } => {
                for (index, value) in args.iter().copied().enumerate() {
                    self.load_value(value, SCRATCH1)?;
                    self.instructions.push(MachineInstr::Store {
                        slot: StackSlot(self.call_slot_offset + CALL_HOME_COUNT + index as u32),
                        src: SCRATCH1,
                    });
                }
                // Save caller-saved allocatable registers before the argument
                // registers (a subset of them) are overwritten, so the values
                // restored after the call are the ones live across it.
                for (index, register) in CALL_HOME_REGISTERS.iter().copied().enumerate() {
                    self.instructions.push(MachineInstr::Store {
                        slot: StackSlot(self.call_slot_offset + index as u32),
                        src: register,
                    });
                }
                for index in 0..args.len() {
                    let slot = StackSlot(self.call_slot_offset + CALL_HOME_COUNT + index as u32);
                    if index < Reg::ARGUMENTS.len() {
                        self.instructions.push(MachineInstr::Load {
                            dst: Reg::ARGUMENTS[index],
                            slot,
                        });
                    } else {
                        self.instructions.push(MachineInstr::Load {
                            dst: SCRATCH1,
                            slot,
                        });
                        self.instructions.push(MachineInstr::StoreArg {
                            src: SCRATCH1,
                            index,
                        });
                    }
                }
                let target = self
                    .symbols
                    .iter()
                    .position(|symbol| symbol == &target)
                    .expect("call symbols are collected before emission")
                    as u32;
                self.instructions.push(MachineInstr::Call { target });
                for (index, register) in CALL_HOME_REGISTERS.iter().copied().enumerate() {
                    self.instructions.push(MachineInstr::Load {
                        dst: register,
                        slot: StackSlot(self.call_slot_offset + index as u32),
                    });
                }
                self.store_definition(dst, Reg::Rax)?;
            }
            VirtualInstr::Return(value) => {
                self.load_value(value, Reg::RETURN)?;
                self.instructions.push(MachineInstr::Ret);
            }
            VirtualInstr::Load { .. } | VirtualInstr::Store { .. } => {
                unreachable!("spill instructions are inserted after register allocation")
            }
        }
        Ok(())
    }

    fn emit_binary(
        &mut self,
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
        kind: BinaryKind,
    ) -> Result<(), CompileError> {
        let work = self.definition_register(dst)?;
        self.load_value(lhs, work)?;
        let rhs = self.operand(rhs, SCRATCH1)?;
        self.instructions.push(match kind {
            BinaryKind::Add => MachineInstr::Add {
                dst: work,
                src: rhs,
            },
            BinaryKind::Sub => MachineInstr::Sub {
                dst: work,
                src: rhs,
            },
            BinaryKind::Mul => MachineInstr::IMul {
                dst: work,
                src: rhs,
            },
            BinaryKind::And => MachineInstr::And {
                dst: work,
                src: rhs,
            },
            BinaryKind::Or => MachineInstr::Or {
                dst: work,
                src: rhs,
            },
            BinaryKind::Xor => MachineInstr::Xor {
                dst: work,
                src: rhs,
            },
        });
        self.store_definition(dst, work)
    }

    fn emit_shift(
        &mut self,
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
        kind: ShiftKind,
    ) -> Result<(), CompileError> {
        if let VirtualValue::Imm(count) = rhs {
            self.load_value(lhs, SCRATCH0)?;
            let count = (count as u64 & 63) as u8;
            self.instructions.push(match kind {
                ShiftKind::Left => MachineInstr::ShlImm {
                    dst: SCRATCH0,
                    count,
                },
                ShiftKind::Right => MachineInstr::ShrImm {
                    dst: SCRATCH0,
                    count,
                },
                ShiftKind::Logical => MachineInstr::LShrImm {
                    dst: SCRATCH0,
                    count,
                },
            });
            return self.store_definition(dst, SCRATCH0);
        }
        self.load_value(lhs, SCRATCH0)?;
        self.load_value(rhs, Reg::Rcx)?;
        self.instructions.push(match kind {
            ShiftKind::Left => MachineInstr::Shl { dst: SCRATCH0 },
            ShiftKind::Right => MachineInstr::Shr { dst: SCRATCH0 },
            ShiftKind::Logical => MachineInstr::LShr { dst: SCRATCH0 },
        });
        self.store_definition(dst, SCRATCH0)
    }

    fn emit_signed_division_power_of_two(
        &mut self,
        dst: VirtualReg,
        lhs: VirtualValue,
        shift: u8,
    ) -> Result<(), CompileError> {
        let work = self.definition_register(dst)?;
        self.load_value(lhs, work)?;
        if shift == 0 {
            return self.store_definition(dst, work);
        }

        self.instructions.push(MachineInstr::Mov {
            dst: SCRATCH1,
            src: work,
        });
        self.instructions.push(MachineInstr::ShrImm {
            dst: SCRATCH1,
            count: 63,
        });
        self.instructions.push(MachineInstr::LShrImm {
            dst: SCRATCH1,
            count: 64 - shift,
        });
        self.instructions.push(MachineInstr::Add {
            dst: work,
            src: Operand::Reg(SCRATCH1),
        });
        self.instructions.push(MachineInstr::ShrImm {
            dst: work,
            count: shift,
        });
        self.store_definition(dst, work)
    }

    fn emit_division(
        &mut self,
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
        result: DivisionResult,
        signed: bool,
    ) -> Result<(), CompileError> {
        let work = self.definition_register(dst)?;
        let rhs = match self.operand(rhs, SCRATCH1)? {
            Operand::Imm(value) => {
                self.instructions.push(MachineInstr::MovImm {
                    dst: SCRATCH1,
                    value: value as i64,
                });
                Operand::Reg(SCRATCH1)
            }
            Operand::Reg(reg @ (Reg::Rax | Reg::Rdx)) => {
                self.instructions.push(MachineInstr::Mov {
                    dst: SCRATCH1,
                    src: reg,
                });
                Operand::Reg(SCRATCH1)
            }
            operand => operand,
        };
        self.load_value(lhs, Reg::Rax)?;
        if signed {
            self.instructions.push(MachineInstr::Cqo);
            self.instructions.push(MachineInstr::IDiv { src: rhs });
        } else {
            self.instructions.push(MachineInstr::Xor {
                dst: Reg::Rdx,
                src: Operand::Reg(Reg::Rdx),
            });
            self.instructions.push(MachineInstr::UDiv { src: rhs });
        }
        self.instructions.push(MachineInstr::Mov {
            dst: work,
            src: match result {
                DivisionResult::Quotient => Reg::Rax,
                DivisionResult::Remainder => Reg::Rdx,
            },
        });
        self.store_definition(dst, work)
    }

    fn definition_register(&mut self, dst: VirtualReg) -> Result<Reg, CompileError> {
        match self.location(dst)? {
            Location::Register(register) => Ok(general_register(register)),
            Location::Spill(_) => Ok(SCRATCH0),
        }
    }

    fn store_definition(&mut self, dst: VirtualReg, source: Reg) -> Result<(), CompileError> {
        match self.location(dst)? {
            Location::Register(register) => {
                let register = general_register(register);
                if register != source {
                    self.instructions.push(MachineInstr::Mov {
                        dst: register,
                        src: source,
                    });
                }
            }
            Location::Spill(slot) => {
                self.instructions.push(MachineInstr::Store {
                    slot: self.stack_slot(slot)?,
                    src: source,
                });
            }
        }
        Ok(())
    }

    fn load_value(&mut self, value: VirtualValue, dst: Reg) -> Result<(), CompileError> {
        match value {
            VirtualValue::Imm(value) => {
                self.instructions.push(MachineInstr::MovImm { dst, value });
            }
            VirtualValue::Reg(virtual_reg) => match self.location(virtual_reg)? {
                Location::Register(source) => {
                    let source = general_register(source);
                    if source != dst {
                        self.instructions
                            .push(MachineInstr::Mov { dst, src: source });
                    }
                }
                Location::Spill(slot) => {
                    self.instructions.push(MachineInstr::Load {
                        dst,
                        slot: self.stack_slot(slot)?,
                    });
                }
            },
        }
        Ok(())
    }

    fn operand(&mut self, value: VirtualValue, scratch: Reg) -> Result<Operand, CompileError> {
        match value {
            VirtualValue::Imm(value) => {
                if let Ok(value) = i32::try_from(value) {
                    Ok(Operand::Imm(value))
                } else {
                    self.instructions.push(MachineInstr::MovImm {
                        dst: scratch,
                        value,
                    });
                    Ok(Operand::Reg(scratch))
                }
            }
            VirtualValue::Reg(virtual_reg) => match self.location(virtual_reg)? {
                Location::Register(register) => Ok(Operand::Reg(general_register(register))),
                Location::Spill(slot) => Ok(Operand::Stack(self.stack_slot(slot)?)),
            },
        }
    }

    fn location(&self, virtual_reg: VirtualReg) -> Result<Location, CompileError> {
        self.allocation
            .allocation(alloc_virtual_reg(virtual_reg))
            .ok_or(CompileError::MissingAllocation(virtual_reg))
    }

    fn stack_slot(&self, slot: SpillSlot) -> Result<StackSlot, CompileError> {
        slot.0
            .checked_add(self.spill_slot_offset)
            .map(StackSlot)
            .ok_or(CompileError::TooManyStackSlots)
    }
}

#[derive(Clone, Copy)]
enum BinaryKind {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy)]
enum ShiftKind {
    Left,
    Right,
    Logical,
}

#[derive(Clone, Copy)]
enum DivisionResult {
    Quotient,
    Remainder,
}

#[cfg(test)]
mod tests;
