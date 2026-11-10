//! Emit allocated operands directly. Fixed constraints create short saved fragments.
use super::*;

#[derive(Clone, Copy, Default)]
struct Protected(u16);
impl Protected {
    fn insert(&mut self, reg: Reg) {
        self.0 |= 1 << reg as u8;
    }
    fn location(&mut self, location: Location) {
        if let Location::Reg(reg) = location {
            self.insert(reg);
        }
    }
    fn contains(self, reg: Reg) -> bool {
        self.0 & (1 << reg as u8) != 0
    }
}

#[derive(Default)]
struct ScratchStats {
    cycle_reg: usize,
    cycle_stack: usize,
    memory_free: usize,
    memory_saved: usize,
    preserve: usize,
    // (origin, instruction kind) -> count of emitted boundary moves.
    moves: std::collections::BTreeMap<(&'static str, &'static str), usize>,
}

pub(super) struct Emitter<'a> {
    allocation: &'a allocation::Allocation,
    function: MachineFunction,
    addresses: Vec<(i32, i32)>,
    saved: HashMap<Reg, StackSlot>,
    scratch: Option<StackSlot>,
    cycle: Option<StackSlot>,
    stats: Option<ScratchStats>,
}
impl<'a> Emitter<'a> {
    pub fn new(
        name: &str,
        allocation: &'a allocation::Allocation,
        layouts: &[SizeAlign],
    ) -> Result<Self, LowerError> {
        let mut function = MachineFunction::new(name);
        let mut bytes = u64::from(allocation.spill_slots) * 8;
        let mut addresses = Vec::new();
        for layout in layouts {
            if !layout.align.is_power_of_two() {
                return Err(LowerError::Unsupported(format!(
                    "stack alignment {}",
                    layout.align
                )));
            }
            let alignment = if layout.align > 16 {
                i32::try_from(layout.align)
                    .map_err(|_| LowerError::Unsupported("stack alignment exceeds i32".into()))?
            } else {
                1
            };
            bytes = bytes
                .checked_add(layout.size.max(1))
                .and_then(|n| n.checked_add(layout.align - 1))
                .ok_or_else(|| LowerError::Unsupported("stack frame overflow".into()))?;
            if alignment == 1 {
                bytes &= !(layout.align - 1);
            }
            let offset = -i32::try_from(bytes)
                .map_err(|_| LowerError::Unsupported("stack frame exceeds i32".into()))?;
            addresses.push((offset, alignment));
        }
        function.stack_slots = u32::try_from(bytes.div_ceil(8))
            .map_err(|_| LowerError::Unsupported("stack frame overflow".into()))?;
        Ok(Self {
            allocation,
            function,
            addresses,
            saved: HashMap::default(),
            scratch: None,
            cycle: None,
            stats: (std::env::var("KFRC_REGALLOC_STATS").as_deref() == Ok("1"))
                .then(ScratchStats::default),
        })
    }
    fn slot(&mut self) -> StackSlot {
        let slot = StackSlot(self.function.stack_slots);
        self.function.stack_slots += 1;
        slot
    }
    fn preserve_slot(&mut self, r: Reg) -> StackSlot {
        if let Some(slot) = self.saved.get(&r) {
            *slot
        } else {
            let slot = self.slot();
            self.saved.insert(r, slot);
            slot
        }
    }
    fn push(&mut self, instruction: MachineInstr) {
        self.function.instructions.push(instruction);
    }
    fn touch(&mut self, reg: Reg) {
        if Reg::CALLEE_SAVED.contains(&reg)
            && reg != Reg::Rbp
            && !self.function.callee_saved.contains(&reg)
        {
            self.function.callee_saved.push(reg);
        }
    }
    fn copy(&mut self, copy: Copy, free_scratch: Option<Reg>) {
        use Location::*;
        if copy.src == copy.dst {
            return;
        }
        if let Reg(r) = copy.dst {
            self.touch(r);
        }
        match (copy.src, copy.dst) {
            (Reg(src), Reg(dst)) => self.push(MachineInstr::Mov { dst, src }),
            (Stack(slot), Reg(dst)) => self.push(MachineInstr::Load { dst, slot }),
            (Imm(value), Reg(dst)) => self.push(MachineInstr::MovImm { dst, value }),
            (Reg(src), Stack(slot)) => self.push(MachineInstr::Store { slot, src }),
            (src, Stack(slot)) => {
                let reg = free_scratch.unwrap_or(super::Reg::R11);
                let saved = if free_scratch.is_none() {
                    if let Some(stats) = &mut self.stats {
                        stats.memory_saved += 1;
                    }
                    let scratch = match self.scratch {
                        Some(s) => s,
                        None => {
                            let s = self.slot();
                            self.scratch = Some(s);
                            s
                        }
                    };
                    self.push(MachineInstr::Store {
                        slot: scratch,
                        src: reg,
                    });
                    Some(scratch)
                } else {
                    if let Some(stats) = &mut self.stats {
                        stats.memory_free += 1;
                    }
                    None
                };
                // The recursive copy has a register destination, so cannot
                // borrow another scratch or overwrite its emergency slot.
                self.copy(Copy { src, dst: Reg(reg) }, None);
                self.push(MachineInstr::Store { slot, src: reg });
                if let Some(slot) = saved {
                    self.push(MachineInstr::Load { slot, dst: reg });
                }
            }
            (_, Imm(_)) => unreachable!("validated copy destination"),
        }
    }
    /// Copies a new definition to its home and, for a value stored at its
    /// definitions, to its memory slot as well.
    fn definition(
        &self,
        copies: &mut Vec<Copy>,
        value: VirtualReg,
        src: Location,
        position: usize,
    ) -> Result<(), LowerError> {
        let home = self.allocation.home(value, position)?;
        copies.push(Copy { src, dst: home });
        if let Some(slot) = self.allocation.memory_slot(value)
            && home != Location::Stack(slot)
        {
            copies.push(Copy {
                src,
                dst: Location::Stack(slot),
            });
        }
        Ok(())
    }
    fn tally(&mut self, origin: &'static str, start: usize) {
        let Some(stats) = &mut self.stats else { return };
        for instruction in &self.function.instructions[start..] {
            let kind = match instruction {
                MachineInstr::Mov { .. } => "mov",
                MachineInstr::Load { .. } => "load",
                MachineInstr::Store { .. } => "store",
                MachineInstr::MovImm { .. } => "imm",
                _ => continue,
            };
            *stats.moves.entry((origin, kind)).or_default() += 1;
        }
    }
    fn free_scratch(&self, protected: Protected) -> Option<Reg> {
        Reg::CALLER_SAVED
            .into_iter()
            .chain(self.function.callee_saved.iter().copied())
            .find(|&reg| reg != Reg::Rbp && !protected.contains(reg))
    }
    fn parallel(&mut self, copies: Vec<Copy>, mut protected: Protected) -> Result<(), LowerError> {
        if copies.is_empty() {
            return Ok(());
        }
        // Identity copies protect values already in their final homes. Collect
        // endpoints before resolve removes them, and keep them for the whole
        // sequence: a consumed source can already contain a final result.
        for copy in &copies {
            protected.location(copy.src);
            protected.location(copy.dst);
        }
        let temporary = self
            .free_scratch(protected)
            .map(Location::Reg)
            .unwrap_or_else(|| {
                Location::Stack(self.cycle.unwrap_or(StackSlot(self.function.stack_slots)))
            });
        let sequence = moves::resolve(copies, temporary)?;
        let uses_temporary = sequence.iter().any(|copy| copy.dst == temporary);
        if uses_temporary {
            match temporary {
                Location::Reg(reg) => {
                    protected.insert(reg);
                    if let Some(stats) = &mut self.stats {
                        stats.cycle_reg +=
                            sequence.iter().filter(|copy| copy.dst == temporary).count();
                    }
                }
                Location::Stack(_) => {
                    if self.cycle.is_none() {
                        self.cycle = Some(self.slot());
                    }
                    if let Some(stats) = &mut self.stats {
                        stats.cycle_stack +=
                            sequence.iter().filter(|copy| copy.dst == temporary).count();
                    }
                }
                Location::Imm(_) => unreachable!(),
            }
        }
        // With no cycle the proposed register is still free for memory copies.
        // With a cycle it must retain the saved value throughout the sequence.
        let scratch = self.free_scratch(protected);
        for copy in sequence {
            self.copy(copy, scratch);
        }
        Ok(())
    }
    pub fn edge(&mut self, source: usize, target: usize) -> Result<(), LowerError> {
        let start = self.function.instructions.len();
        let copies = self.allocation.edge(source, target)?;
        if !copies.is_empty() {
            let mut protected = Protected::default();
            // edge() omits identity copies, so include every live-in home,
            // including values whose location does not change on this edge.
            for value in self.allocation.edge_values(source, target) {
                protected.location(self.allocation.home(value, source)?);
                protected.location(self.allocation.home(value, target)?);
            }
            self.parallel(copies, protected)?;
        }
        let adjacent = source + 1 == target;
        self.tally(if adjacent { "edge" } else { "jump-edge" }, start);
        checker::edge(
            self.allocation,
            source,
            target,
            &self.function.instructions[start..],
        )
    }
    pub fn edge_block(
        &mut self,
        label: u32,
        source: usize,
        target: usize,
        destination: u32,
    ) -> Result<(), LowerError> {
        self.push(MachineInstr::Label(label));
        self.edge(source, target)?;
        self.push(MachineInstr::Jmp(destination));
        Ok(())
    }
    pub fn instruction(&mut self, inst: &Instruction, position: usize) -> Result<(), LowerError> {
        let start = self.function.instructions.len();
        match &inst.operation {
            Operation::Entry(args) => {
                let mut copies = Vec::new();
                for (dst, index) in args {
                    self.definition(
                        &mut copies,
                        *dst,
                        Location::Reg(Reg::ARGUMENTS[*index]),
                        position,
                    )?;
                }
                let mut protected = Protected::default();
                for reg in Reg::ARGUMENTS {
                    protected.insert(reg);
                }
                self.parallel(copies, protected)?;
                self.tally("entry", start);
                checker::entry(
                    self.allocation,
                    args,
                    position,
                    &self.function.instructions[start..],
                )?;
                return Ok(());
            }
            Operation::Parallel(copies) => {
                let definitions: Vec<_> = copies.iter().map(|(dst, _)| *dst).collect();
                let mut protected = Protected::default();
                for value in self.allocation.live_through(position, &definitions) {
                    protected.location(self.allocation.home(value, position)?);
                }
                let mut moves = Vec::new();
                for (dst, src) in copies {
                    let src = self.allocation.value(*src, position)?;
                    self.definition(&mut moves, *dst, src, position)?;
                }
                let copies = moves;
                self.parallel(copies, protected)?;
                self.tally("phi", start);
                checker::parallel(
                    self.allocation,
                    inst,
                    position,
                    &self.function.instructions[start..],
                )?;
                return Ok(());
            }
            _ => {}
        }
        let registers = self.allocation.registers(inst, position)?;
        let inputs = &registers.inputs;
        let outputs = &registers.outputs;
        let before = inst
            .inputs
            .iter()
            .zip(inputs)
            .map(|(input, &reg)| {
                Ok(Copy {
                    src: self.allocation.value(input.value, position)?,
                    dst: Location::Reg(reg),
                })
            })
            .collect::<Result<Vec<_>, LowerError>>()?;
        let mut overwritten = inst.clobbers.clone();
        overwritten.extend(outputs);
        overwritten.extend(
            before
                .iter()
                .filter(|c| c.src != c.dst)
                .filter_map(|c| c.dst.register()),
        );
        let defs: Vec<_> = inst.outputs.iter().map(|o| o.value).collect();
        let mut restores = Vec::new();
        let mut protected = Protected::default();
        for v in self.allocation.live_through(position, &defs) {
            if let Location::Reg(r) = self.allocation.home(v, position)? {
                protected.insert(r);
                if overwritten.contains(&r) {
                    let slot = self.preserve_slot(r);
                    // These direct stores cannot borrow scratch. Finish all
                    // saves before operand moves overwrite any old value.
                    self.copy(
                        Copy {
                            src: Location::Reg(r),
                            dst: Location::Stack(slot),
                        },
                        None,
                    );
                    if let Some(stats) = &mut self.stats {
                        stats.preserve += 1;
                    }
                    restores.push(Copy {
                        src: Location::Stack(slot),
                        dst: Location::Reg(r),
                    });
                }
            }
        }
        // Repeated operands may demand the same register. Keep one assignment
        // only when both sources agree; conflicting fixed constraints are errors.
        let mut unique = Vec::<Copy>::new();
        for copy in before {
            if let Some(old) = unique.iter().find(|c| c.dst == copy.dst) {
                if old.src != copy.src {
                    return Err(LowerError::Malformed(
                        "conflicting operand constraints".into(),
                    ));
                }
            } else {
                unique.push(copy);
            }
        }
        self.parallel(unique, protected)?;
        let op_start = self.function.instructions.len();
        self.tally(operand_origin(&inst.operation), start);
        for &r in outputs {
            self.touch(r);
        }
        self.operation(&inst.operation, inputs, outputs)?;
        let op_end = self.function.instructions.len();
        let mut after = Vec::new();
        for (output, &reg) in inst.outputs.iter().zip(outputs) {
            self.definition(&mut after, output.value, Location::Reg(reg), position)?;
        }
        // Results and saved old values already exist simultaneously. Resolve
        // them together so a restore cannot overwrite an unconsumed result.
        after.extend(restores);
        self.parallel(after, protected)?;
        self.tally("result", op_end);
        checker::instruction(
            self.allocation,
            inst,
            position,
            inputs,
            outputs,
            &self.function.instructions[start..op_start],
            &self.function.instructions[op_end..],
        )
        .map_err(|error| match error {
            LowerError::Malformed(message) => LowerError::Malformed(format!(
                "{message} at instruction {position} {:?}",
                inst.operation
            )),
            other => other,
        })?;
        Ok(())
    }
    fn symbol(&mut self, symbol: &str) -> u32 {
        if let Some(index) = self.function.symbols.iter().position(|s| s == symbol) {
            index as u32
        } else {
            let index = self.function.symbols.len() as u32;
            self.function.symbols.push(symbol.into());
            index
        }
    }
    fn operation(
        &mut self,
        op: &Operation,
        input: &[Reg],
        output: &[Reg],
    ) -> Result<(), LowerError> {
        use Operation::*;
        match op {
            Copy => {}
            Unary(neg) => self.push(if *neg {
                MachineInstr::Neg { dst: output[0] }
            } else {
                MachineInstr::Not { dst: output[0] }
            }),
            Binary(op) => {
                let dst = output[0];
                let src = MachineOperand::Reg(input[1]);
                self.push(match op {
                    constraints::Binary::Add => MachineInstr::Add { dst, src },
                    constraints::Binary::Sub => MachineInstr::Sub { dst, src },
                    constraints::Binary::Mul => MachineInstr::IMul { dst, src },
                    constraints::Binary::And => MachineInstr::And { dst, src },
                    constraints::Binary::Or => MachineInstr::Or { dst, src },
                    constraints::Binary::Xor => MachineInstr::Xor { dst, src },
                });
            }
            BinaryImm(op, value) if input[0] != output[0] => {
                let offset = constraints::offset_address(*op, *value)
                    .ok_or_else(|| LowerError::Malformed("two-address operand mismatch".into()))?;
                self.push(MachineInstr::OffsetAddress {
                    dst: output[0],
                    base: input[0],
                    offset,
                })
            }
            BinaryImm(op, value) => {
                let dst = output[0];
                let src = MachineOperand::Imm(*value);
                self.push(match op {
                    constraints::Binary::Add => MachineInstr::Add { dst, src },
                    constraints::Binary::Sub => MachineInstr::Sub { dst, src },
                    constraints::Binary::Mul => MachineInstr::IMul { dst, src },
                    constraints::Binary::And => MachineInstr::And { dst, src },
                    constraints::Binary::Or => MachineInstr::Or { dst, src },
                    constraints::Binary::Xor => MachineInstr::Xor { dst, src },
                });
            }
            Shift(op, immediate) => {
                let dst = output[0];
                self.push(match (op, immediate) {
                    (constraints::Shift::Left, Some(count)) => {
                        MachineInstr::ShlImm { dst, count: *count }
                    }
                    (constraints::Shift::Arithmetic, Some(count)) => {
                        MachineInstr::ShrImm { dst, count: *count }
                    }
                    (constraints::Shift::Logical, Some(count)) => {
                        MachineInstr::LShrImm { dst, count: *count }
                    }
                    (constraints::Shift::Left, None) => MachineInstr::Shl { dst },
                    (constraints::Shift::Arithmetic, None) => MachineInstr::Shr { dst },
                    (constraints::Shift::Logical, None) => MachineInstr::LShr { dst },
                });
            }
            Rotate(width, right, count) => self.push(MachineInstr::Rotate {
                dst: output[0],
                width: *width,
                right: *right,
                count: *count,
            }),
            Extend(width, signed) => self.push(MachineInstr::Extend {
                dst: output[0],
                src: input[0],
                width: *width,
                signed: *signed,
            }),
            ByteSwap(wide) => self.push(MachineInstr::ByteSwap {
                dst: output[0],
                wide: *wide,
            }),
            BitScanZero(reverse) => self.push(MachineInstr::BitScanZero {
                dst: output[0],
                src: input[0],
                zero: input[1],
                reverse: *reverse,
            }),
            Divide { signed } => {
                if *signed {
                    self.push(MachineInstr::Cqo);
                    self.push(MachineInstr::IDiv {
                        src: MachineOperand::Reg(input[1]),
                    });
                } else {
                    self.push(MachineInstr::Xor {
                        dst: Reg::Rdx,
                        src: MachineOperand::Reg(Reg::Rdx),
                    });
                    self.push(MachineInstr::UDiv {
                        src: MachineOperand::Reg(input[1]),
                    });
                }
            }
            MulHigh => self.push(MachineInstr::MulHigh { src: input[1] }),
            Compare(condition) => {
                self.push(MachineInstr::Cmp {
                    lhs: input[0],
                    rhs: MachineOperand::Reg(input[1]),
                });
                self.push(MachineInstr::SetCc {
                    condition: *condition,
                    dst: output[0],
                });
            }
            CompareImm(condition, value) => {
                self.push(MachineInstr::Cmp {
                    lhs: input[0],
                    rhs: MachineOperand::Imm(*value),
                });
                self.push(MachineInstr::SetCc {
                    condition: *condition,
                    dst: output[0],
                });
            }
            TestImm(value) => self.push(MachineInstr::Cmp {
                lhs: input[0],
                rhs: MachineOperand::Imm(*value),
            }),
            TestLowByte => self.push(MachineInstr::TestByte { reg: input[0] }),
            Test(zero) => self.push(if *zero {
                MachineInstr::Test { reg: input[0] }
            } else {
                MachineInstr::Cmp {
                    lhs: input[0],
                    rhs: MachineOperand::Reg(input[1]),
                }
            }),
            StackAddress(slot) => {
                let &(offset, alignment) = self
                    .addresses
                    .get(*slot)
                    .ok_or_else(|| LowerError::Malformed("invalid stack address".into()))?;
                let dst = output[0];
                self.push(MachineInstr::StackAddress { dst, offset });
                if alignment > 1 {
                    self.push(MachineInstr::Add {
                        dst,
                        src: MachineOperand::Imm(alignment - 1),
                    });
                    self.push(MachineInstr::And {
                        dst,
                        src: MachineOperand::Imm(-alignment),
                    });
                }
            }
            SymbolAddress(symbol, addend) => {
                let symbol = self.symbol(symbol);
                self.push(MachineInstr::SymbolAddress {
                    dst: output[0],
                    symbol,
                    addend: *addend,
                });
            }
            ThreadLocalAddress(symbol) => {
                let symbol = self.symbol(symbol);
                self.push(MachineInstr::ThreadLocalAddress {
                    dst: output[0],
                    symbol,
                });
            }
            Load { bytes, offset } => self.push(MachineInstr::LoadIndirect {
                dst: output[0],
                address: input[0],
                offset: *offset,
                bytes: *bytes,
            }),
            Store { bytes, offset } => self.push(MachineInstr::StoreIndirect {
                address: input[0],
                offset: *offset,
                src: input[1],
                bytes: *bytes,
            }),
            StoreImmediate {
                bytes,
                offset,
                value,
            } => self.push(MachineInstr::StoreImmediate {
                address: input[0],
                offset: *offset,
                value: *value,
                bytes: *bytes,
            }),
            LoadSlot(slot) => self.push(MachineInstr::Load {
                dst: output[0],
                slot: *slot,
            }),
            StoreSlot(slot) => self.push(MachineInstr::Store {
                src: input[0],
                slot: *slot,
            }),
            Incoming(index) => self.push(MachineInstr::LoadArg {
                dst: output[0],
                index: *index,
            }),
            Outgoing(index) => self.push(MachineInstr::StoreArg {
                src: input[0],
                index: *index,
            }),
            LoadVector(dst) => self.push(MachineInstr::LoadVector {
                dst: *dst,
                address: input[0],
            }),
            StoreVector(src) => self.push(MachineInstr::StoreVector {
                src: *src,
                address: input[0],
            }),
            MulOverflow { signed, bits } => self.push(MachineInstr::MulOverflow {
                dst: output[0],
                lhs: input[0],
                rhs: input[1],
                signed: *signed,
                bits: *bits,
            }),
            Call(symbol) => {
                if let Some(symbol) = symbol {
                    let target = self.symbol(symbol);
                    self.push(MachineInstr::Call { target });
                } else {
                    self.push(MachineInstr::CallIndirect {
                        target: *input.last().unwrap(),
                    });
                }
            }
            Label(label) => self.push(MachineInstr::Label(*label)),
            Jump(label) => self.push(MachineInstr::Jmp(*label)),
            Branch(condition, target) => self.push(MachineInstr::Jcc {
                condition: *condition,
                target: *target,
            }),
            Return => self.push(MachineInstr::Ret),
            Entry(_) | Parallel(_) => unreachable!(),
        }
        Ok(())
    }
    pub fn finish(mut self) -> MachineFunction {
        if let Some(stats) = &self.stats {
            eprintln!(
                "kfrc-regalloc-stats symbol={} cycle_reg={} cycle_stack={} memory_free={} memory_saved={} preserve={}",
                self.function.name,
                stats.cycle_reg,
                stats.cycle_stack,
                stats.memory_free,
                stats.memory_saved,
                stats.preserve
            );
            let moves: Vec<_> = stats
                .moves
                .iter()
                .map(|((origin, kind), count)| format!("{origin}.{kind}={count}"))
                .collect();
            eprintln!(
                "kfrc-regalloc-moves symbol={} {}",
                self.function.name,
                moves.join(" ")
            );
        }
        self.function
            .callee_saved
            .sort_by_key(|r| Reg::ALLOCATABLE.iter().position(|v| v == r));
        self.function
    }
}

fn operand_origin(operation: &Operation) -> &'static str {
    match operation {
        Operation::Call(_) => "call-operand",
        Operation::Return => "return-operand",
        Operation::Copy => "copy-operand",
        _ => "operand",
    }
}

#[cfg(test)]
#[path = "scratch_tests.rs"]
mod scratch_tests;
