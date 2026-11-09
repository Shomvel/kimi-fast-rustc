//! Allocation for the memory instruction stream. Fixed scratch registers stay
//! outside the pool; values in the pool survive SysV calls without shuffles.
use crate::analysis::liveness::{self, InstructionLiveness};
use crate::lower::LowerError;
use crate::machine::regalloc::{Location, RegisterAllocator};
use crate::machine::x86::{MachineInstr, Reg, StackSlot, VirtualReg, VirtualValue};

const REGISTERS: [Reg; 5] = [Reg::Rbx, Reg::R12, Reg::R13, Reg::R14, Reg::R15];

pub(crate) struct MemoryAllocation {
    locations: Vec<Option<Location>>,
    pub spill_slots: u32,
    pub callee_saved: Vec<Reg>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::regalloc::{PhysicalRegister, SpillSlot};

    #[test]
    fn copies_use_exact_locations_and_only_flag_preserving_instructions() {
        let allocation = MemoryAllocation {
            locations: vec![
                Some(Location::Register(PhysicalRegister(0))),
                Some(Location::Register(PhysicalRegister(0))),
                Some(Location::Register(PhysicalRegister(1))),
                Some(Location::Spill(SpillSlot(0))),
                Some(Location::Spill(SpillSlot(0))),
                Some(Location::Spill(SpillSlot(1))),
            ],
            spill_slots: 2,
            callee_saved: vec![Reg::Rbx, Reg::R12],
        };
        let cases = [
            (0, VirtualValue::Reg(VirtualReg(1)), vec![]),
            (3, VirtualValue::Reg(VirtualReg(4)), vec![]),
            (
                2,
                VirtualValue::Reg(VirtualReg(0)),
                vec![MachineInstr::Mov {
                    dst: Reg::R12,
                    src: Reg::Rbx,
                }],
            ),
            (
                3,
                VirtualValue::Reg(VirtualReg(0)),
                vec![MachineInstr::Store {
                    slot: StackSlot(0),
                    src: Reg::Rbx,
                }],
            ),
            (
                0,
                VirtualValue::Reg(VirtualReg(3)),
                vec![MachineInstr::Load {
                    dst: Reg::Rbx,
                    slot: StackSlot(0),
                }],
            ),
            (
                5,
                VirtualValue::Reg(VirtualReg(3)),
                vec![
                    MachineInstr::Load {
                        dst: Reg::Rax,
                        slot: StackSlot(0),
                    },
                    MachineInstr::Store {
                        slot: StackSlot(1),
                        src: Reg::Rax,
                    },
                ],
            ),
            (
                0,
                VirtualValue::Imm(-7),
                vec![MachineInstr::MovImm {
                    dst: Reg::Rbx,
                    value: -7,
                }],
            ),
            (
                3,
                VirtualValue::Imm(-7),
                vec![
                    MachineInstr::MovImm {
                        dst: Reg::Rax,
                        value: -7,
                    },
                    MachineInstr::Store {
                        slot: StackSlot(0),
                        src: Reg::Rax,
                    },
                ],
            ),
        ];
        for (dst, src, expected) in cases {
            let mut instructions = Vec::new();
            allocation.copy(&mut instructions, VirtualReg(dst), src);
            assert_eq!(instructions, expected);
        }
    }
}

impl MemoryAllocation {
    pub fn new(facts: &[InstructionLiveness], virtual_count: u32) -> Result<Self, LowerError> {
        let intervals = liveness::intervals(facts);
        let allocation = RegisterAllocator::new(REGISTERS.len() as u16)
            .allocate(&intervals)
            .map_err(|error| LowerError::Malformed(format!("memory allocation: {error}")))?;
        let mut locations = vec![None; virtual_count as usize];
        let mut used = [false; REGISTERS.len()];
        for (register, location) in allocation.iter() {
            locations[register.0 as usize] = Some(location);
            if let Location::Register(physical) = location {
                used[physical.index()] = true;
            }
        }
        Ok(Self {
            locations,
            spill_slots: allocation.spill_slot_count,
            callee_saved: REGISTERS
                .into_iter()
                .zip(used)
                .filter_map(|(register, used)| used.then_some(register))
                .collect(),
        })
    }

    fn location(&self, register: VirtualReg) -> Location {
        self.locations[register.0 as usize].expect("selected register has a live interval")
    }

    /// Copy allocated values without routing register moves through a scratch.
    /// Every emitted instruction preserves flags. RAX is outside the pool and
    /// is needed only for immediate-to-spill and distinct spill-to-spill copies.
    pub fn copy(&self, output: &mut Vec<MachineInstr>, dst: VirtualReg, src: VirtualValue) {
        let destination = self.location(dst);
        let source = match src {
            VirtualValue::Reg(register) => {
                let source = self.location(register);
                if source == destination {
                    return;
                }
                Some(source)
            }
            VirtualValue::Imm(_) => None,
        };
        match (destination, source) {
            (Location::Register(dst), _) => self.load(output, src, REGISTERS[dst.index()]),
            (Location::Spill(dst), Some(Location::Register(src))) => {
                output.push(MachineInstr::Store {
                    slot: StackSlot(dst.0),
                    src: REGISTERS[src.index()],
                });
            }
            (Location::Spill(dst), _) => {
                self.load(output, src, Reg::Rax);
                output.push(MachineInstr::Store {
                    slot: StackSlot(dst.0),
                    src: Reg::Rax,
                });
            }
        }
    }

    pub fn load(&self, output: &mut Vec<MachineInstr>, value: VirtualValue, dst: Reg) {
        match value {
            VirtualValue::Imm(value) => output.push(MachineInstr::MovImm { dst, value }),
            VirtualValue::Reg(register) => match self.location(register) {
                Location::Register(register) => {
                    let src = REGISTERS[register.index()];
                    if src != dst {
                        output.push(MachineInstr::Mov { dst, src });
                    }
                }
                Location::Spill(slot) => output.push(MachineInstr::Load {
                    dst,
                    slot: StackSlot(slot.0),
                }),
            },
        }
    }

    pub fn save(&self, output: &mut Vec<MachineInstr>, register: VirtualReg, src: Reg) {
        match self.location(register) {
            Location::Register(register) => {
                let dst = REGISTERS[register.index()];
                if src != dst {
                    output.push(MachineInstr::Mov { dst, src });
                }
            }
            Location::Spill(slot) => output.push(MachineInstr::Store {
                slot: StackSlot(slot.0),
                src,
            }),
        }
    }
}
