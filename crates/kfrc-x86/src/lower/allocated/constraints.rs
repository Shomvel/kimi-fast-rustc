//! Explicit operand timing and architectural constraints for the shared stream.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Constraint {
    Any,
    Fixed(Reg),
    Reuse(usize),
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Phase {
    Early,
    Late,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Input {
    pub value: VirtualValue,
    pub constraint: Constraint,
    pub phase: Phase,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Output {
    pub value: VirtualReg,
    pub constraint: Constraint,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Binary {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum Shift {
    Left,
    Arithmetic,
    Logical,
}

#[derive(Clone, Debug)]
pub(super) enum Operation {
    Copy,
    Entry(Vec<(VirtualReg, usize)>),
    Parallel(Vec<(VirtualReg, VirtualValue)>),
    Unary(bool),
    Binary(Binary),
    /// Binary operation whose right operand is a sign-extended imm32.
    BinaryImm(Binary, i32),
    Shift(Shift, Option<u8>),
    Rotate(BitWidth, bool, Option<u8>),
    ByteSwap(bool),
    Extend(BitWidth, bool),
    BitScanZero(bool),
    Divide {
        signed: bool,
    },
    MulHigh,
    Compare(Condition),
    CompareImm(Condition, i32),
    Test(bool),
    TestImm(i32),
    TestLowByte,
    StackAddress(usize),
    SymbolAddress(String, i64),
    ThreadLocalAddress(String),
    Load {
        bytes: u8,
        offset: i32,
    },
    Store {
        bytes: u8,
        offset: i32,
    },
    StoreImmediate {
        bytes: u8,
        offset: i32,
        value: i32,
    },
    LoadSlot(StackSlot),
    StoreSlot(StackSlot),
    Incoming(usize),
    Outgoing(usize),
    LoadVector(Xmm),
    StoreVector(Xmm),
    MulOverflow {
        signed: bool,
        bits: u8,
    },
    Call(Option<String>),
    Label(u32),
    Jump(u32),
    Branch(Condition, u32),
    Return,
}

#[derive(Clone, Debug)]
pub(super) struct Instruction {
    pub operation: Operation,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub clobbers: Vec<Reg>,
}

impl Instruction {
    fn new(operation: Operation) -> Self {
        Self {
            operation,
            inputs: Vec::new(),
            outputs: Vec::new(),
            clobbers: Vec::new(),
        }
    }
    fn input(&mut self, value: VirtualValue, constraint: Constraint, phase: Phase) {
        self.inputs.push(Input {
            value,
            constraint,
            phase,
        });
    }
    fn output(&mut self, value: VirtualReg, constraint: Constraint) {
        self.outputs.push(Output { value, constraint });
    }
    pub(super) fn facts(&self) -> InstructionLiveness {
        let mut facts = InstructionLiveness::default();
        facts
            .uses
            .extend(self.inputs.iter().filter_map(|input| match input.value {
                VirtualValue::Reg(v) => Some(v),
                _ => None,
            }));
        facts
            .definitions
            .extend(self.outputs.iter().map(|output| output.value));
        match &self.operation {
            Operation::Entry(args) => facts.definitions.extend(args.iter().map(|(v, _)| *v)),
            Operation::Parallel(copies) => {
                facts.definitions.extend(copies.iter().map(|(v, _)| *v));
                facts
                    .uses
                    .extend(copies.iter().filter_map(|(_, v)| match v {
                        VirtualValue::Reg(v) => Some(*v),
                        _ => None,
                    }));
            }
            Operation::Label(label) => facts.flow = Flow::Label(*label),
            Operation::Jump(label) => facts.flow = Flow::Jump(*label),
            Operation::Branch(_, label) => facts.flow = Flow::Branch(*label),
            Operation::Return => facts.flow = Flow::Return,
            _ => {}
        }
        facts
    }
}

pub(super) fn select(stream: Vec<LowerInstr>) -> Vec<Instruction> {
    use Constraint::*;
    use Phase::*;
    let mut output = Vec::new();
    let mut entry = Vec::new();
    // A flags producer whose operands were swapped to put an immediate on the
    // right; the branch consuming its flags reads the mirrored condition.
    let mut mirror_branch = false;
    for instruction in stream {
        let mut selected = match instruction {
            LowerInstr::ParallelCopy(copies) => Instruction::new(Operation::Parallel(copies)),
            LowerInstr::Scalar(instruction) => {
                use VirtualInstr::*;
                match instruction {
                    LoadArg { dst, index } if index < 6 => {
                        entry.push((dst, index));
                        continue;
                    }
                    LoadArg { dst, index } => def(Operation::Incoming(index), dst),
                    Mov { dst, src } => unary(Operation::Copy, dst, src),
                    Neg { dst, src } => unary(Operation::Unary(true), dst, src),
                    Not { dst, src } => unary(Operation::Unary(false), dst, src),
                    Add { dst, lhs, rhs } => binary(Binary::Add, dst, lhs, rhs),
                    Sub { dst, lhs, rhs } => binary(Binary::Sub, dst, lhs, rhs),
                    Mul { dst, lhs, rhs } => binary(Binary::Mul, dst, lhs, rhs),
                    And { dst, lhs, rhs } => binary(Binary::And, dst, lhs, rhs),
                    Or { dst, lhs, rhs } => binary(Binary::Or, dst, lhs, rhs),
                    Xor { dst, lhs, rhs } => binary(Binary::Xor, dst, lhs, rhs),
                    Shl { dst, lhs, rhs } => shift(Shift::Left, dst, lhs, rhs),
                    Shr { dst, lhs, rhs } => shift(Shift::Arithmetic, dst, lhs, rhs),
                    LShr { dst, lhs, rhs } => shift(Shift::Logical, dst, lhs, rhs),
                    Div { dst, lhs, rhs } => divide(true, false, dst, lhs, rhs),
                    Rem { dst, lhs, rhs } => divide(true, true, dst, lhs, rhs),
                    UDiv { dst, lhs, rhs } => divide(false, false, dst, lhs, rhs),
                    URem { dst, lhs, rhs } => divide(false, true, dst, lhs, rhs),
                    MulHigh { dst, lhs, rhs } => {
                        let mut i = Instruction::new(Operation::MulHigh);
                        i.input(lhs, Constraint::Fixed(Reg::Rax), Phase::Early);
                        i.input(rhs, Constraint::Any, Phase::Late);
                        i.output(dst, Constraint::Fixed(Reg::Rdx));
                        i.clobbers = vec![Reg::Rax, Reg::Rdx];
                        i
                    }
                    Compare {
                        dst,
                        condition,
                        lhs,
                        rhs,
                    } => {
                        let (condition, lhs, rhs) = immediate_right(condition, lhs, rhs);
                        let mut inst = match immediate(rhs) {
                            Some(value) => {
                                Instruction::new(Operation::CompareImm(condition, value))
                            }
                            None => Instruction::new(Operation::Compare(condition)),
                        };
                        inst.input(lhs, Any, Early);
                        if immediate(rhs).is_none() {
                            inst.input(rhs, Any, Early);
                        }
                        inst.output(dst, Any);
                        inst
                    }
                    Test { lhs, rhs } => {
                        let swapped = immediate(lhs).is_some() && immediate(rhs).is_none();
                        let (lhs, rhs) = if swapped { (rhs, lhs) } else { (lhs, rhs) };
                        mirror_branch = swapped;
                        let zero = rhs == VirtualValue::Imm(0);
                        let mut inst = match immediate(rhs) {
                            Some(value) if !zero => Instruction::new(Operation::TestImm(value)),
                            _ => Instruction::new(Operation::Test(zero)),
                        };
                        inst.input(lhs, Any, Early);
                        if !zero && immediate(rhs).is_none() {
                            inst.input(rhs, Any, Early);
                        }
                        inst
                    }
                    Load { dst, slot } => def(Operation::LoadSlot(slot), dst),
                    Store { slot, src } => use_one(Operation::StoreSlot(slot), src),
                    Label(label) => Instruction::new(Operation::Label(label)),
                    Jmp(label) => Instruction::new(Operation::Jump(label)),
                    Jcc { condition, target } => {
                        let condition = if std::mem::take(&mut mirror_branch) {
                            condition.swapped()
                        } else {
                            condition
                        };
                        Instruction::new(Operation::Branch(condition, target))
                    }
                    Return(value) => ret(&[value]),
                    Call { dst, target, args } => {
                        call(
                            &mut output,
                            vec![dst],
                            CallDestination::Direct(target),
                            args.into_iter().enumerate().collect(),
                        );
                        continue;
                    }
                    Phi { .. } | Nop => continue,
                }
            }
            LowerInstr::Rotate {
                dst,
                value,
                amount,
                width,
                right,
            } => {
                let immediate = match amount {
                    VirtualValue::Imm(n) => Some((n as u8) % (width.bits() as u8)),
                    _ => None,
                };
                let mut inst = unary(Operation::Rotate(width, right, immediate), dst, value);
                if immediate.is_none() {
                    inst.input(amount, Fixed(Reg::Rcx), Late);
                }
                inst
            }
            LowerInstr::ByteSwap { dst, value, wide } => {
                unary(Operation::ByteSwap(wide), dst, value)
            }
            LowerInstr::Extend {
                dst,
                value,
                width,
                signed,
            } => {
                // movzx/movsx read any register and write any other.
                let mut inst = Instruction::new(Operation::Extend(width, signed));
                inst.input(VirtualValue::Reg(value), Any, Early);
                inst.output(dst, Any);
                inst
            }
            LowerInstr::TestLowByte { value } => {
                mirror_branch = false;
                let mut inst = Instruction::new(Operation::TestLowByte);
                inst.input(VirtualValue::Reg(value), Any, Early);
                inst
            }
            LowerInstr::BitScanZero {
                dst,
                value,
                zero,
                reverse,
            } => {
                let mut inst = Instruction::new(Operation::BitScanZero(reverse));
                inst.input(value, Any, Early);
                inst.input(zero, Any, Late);
                // BSF/BSR may overwrite the early source, but the fallback
                // must survive until CMOVZ, including source == fallback.
                inst.output(dst, Any);
                inst
            }
            LowerInstr::StackAddress { dst, slot } => def(Operation::StackAddress(slot), dst),
            LowerInstr::SymbolAddress {
                dst,
                symbol,
                addend,
            } => def(Operation::SymbolAddress(symbol, addend), dst),
            LowerInstr::ThreadLocalAddress { dst, symbol } => {
                def(Operation::ThreadLocalAddress(symbol), dst)
            }
            LowerInstr::Load {
                dst,
                address,
                offset,
                bytes,
            } => {
                let mut inst = Instruction::new(Operation::Load { bytes, offset });
                inst.input(address, Any, Early);
                inst.output(dst, Any);
                inst
            }
            LowerInstr::Store {
                address,
                offset,
                src,
                bytes,
            } => {
                if let Some(value) = store_immediate(src, bytes) {
                    let mut inst = Instruction::new(Operation::StoreImmediate {
                        bytes,
                        offset,
                        value,
                    });
                    inst.input(address, Any, Early);
                    output.push(inst);
                    continue;
                }
                let mut inst = Instruction::new(Operation::Store { bytes, offset });
                inst.input(address, Any, Early);
                inst.input(src, Any, Early);
                inst
            }
            LowerInstr::LoadVector { dst, address } => use_one(Operation::LoadVector(dst), address),
            LowerInstr::StoreVector { address, src } => {
                use_one(Operation::StoreVector(src), address)
            }
            LowerInstr::MulOverflow {
                dst,
                lhs,
                rhs,
                signed,
                bits,
            } => {
                let mut inst = Instruction::new(Operation::MulOverflow { signed, bits });
                inst.input(lhs, Any, Early);
                inst.input(rhs, Any, Late);
                inst.output(dst, Reuse(0));
                inst.clobbers = vec![Reg::Rax, Reg::Rdx];
                inst
            }
            LowerInstr::ReturnPair { first, second } => ret(&[first, second]),
            LowerInstr::CallPair {
                first,
                second,
                target,
                args,
            } => {
                call(
                    &mut output,
                    vec![first, second],
                    target,
                    args.into_iter().enumerate().collect(),
                );
                continue;
            }
            LowerInstr::CallIndirect { dst, target, args } => {
                call(
                    &mut output,
                    vec![dst],
                    CallDestination::Indirect(target),
                    args.into_iter().enumerate().collect(),
                );
                continue;
            }
            LowerInstr::AbiCall {
                results,
                target,
                args,
                vector_args,
                vector_result,
            } => {
                for (xmm, address) in vector_args {
                    output.push(use_one(Operation::LoadVector(xmm), address));
                }
                call(&mut output, results, target, args);
                if let Some(address) = vector_result {
                    output.push(use_one(Operation::StoreVector(Xmm(0)), address));
                }
                continue;
            }
        };
        // Reuse constraints make two-address writes explicit. Sources needed
        // after the early read cannot be overwritten by the output.
        for out in &selected.outputs {
            if let Reuse(index) = out.constraint {
                selected.inputs[index].phase = Early;
            }
        }
        output.push(selected);
    }
    if !entry.is_empty() {
        output.insert(0, Instruction::new(Operation::Entry(entry)));
    }
    replace_undefined_uses(eliminate_copies(output))
}
fn def(op: Operation, dst: VirtualReg) -> Instruction {
    let mut i = Instruction::new(op);
    i.output(dst, Constraint::Any);
    i
}
fn use_one(op: Operation, src: VirtualValue) -> Instruction {
    let mut i = Instruction::new(op);
    i.input(src, Constraint::Any, Phase::Early);
    i
}
fn unary(op: Operation, dst: VirtualReg, src: VirtualValue) -> Instruction {
    let mut i = use_one(op, src);
    i.output(dst, Constraint::Reuse(0));
    i
}
/// A constant usable as the sign-extended imm32 operand of a 64-bit instruction.
fn immediate(value: VirtualValue) -> Option<i32> {
    match value {
        VirtualValue::Imm(n) => i32::try_from(n).ok(),
        VirtualValue::Reg(_) => None,
    }
}
/// The immediate operand of `mov [address + offset], imm` storing `src`, when
/// that form is no longer than materializing the constant in a register: a
/// zero of four or eight bytes is shorter through a `xor` register.
fn store_immediate(src: VirtualValue, bytes: u8) -> Option<i32> {
    let VirtualValue::Imm(n) = src else {
        return None;
    };
    match bytes {
        1 => Some(i32::from(n as i8)),
        2 => Some(i32::from(n as i16)),
        4 if n as i32 != 0 => Some(n as i32),
        8 if n != 0 => i32::try_from(n).ok(),
        _ => None,
    }
}
/// The displacement of `lea dst, [src + offset]` computing `src op value`.
pub(super) fn offset_address(op: Binary, value: i32) -> Option<i32> {
    match op {
        Binary::Add => Some(value),
        Binary::Sub => value.checked_neg(),
        _ => None,
    }
}
/// Puts an encodable immediate on the right of a comparison, where `cmp`
/// accepts it, mirroring the condition.
fn immediate_right(
    condition: Condition,
    lhs: VirtualValue,
    rhs: VirtualValue,
) -> (Condition, VirtualValue, VirtualValue) {
    if immediate(lhs).is_some() && immediate(rhs).is_none() {
        (condition.swapped(), rhs, lhs)
    } else {
        (condition, lhs, rhs)
    }
}
fn binary(op: Binary, dst: VirtualReg, lhs: VirtualValue, rhs: VirtualValue) -> Instruction {
    let commutative = !matches!(op, Binary::Sub);
    let (lhs, rhs) = if commutative && immediate(lhs).is_some() && immediate(rhs).is_none() {
        (rhs, lhs)
    } else {
        (lhs, rhs)
    };
    if let Some(value) = immediate(rhs) {
        let mut i = unary(Operation::BinaryImm(op, value), dst, lhs);
        if offset_address(op, value).is_some() {
            // `lea` writes any register, so the source may stay live in place.
            i.outputs[0].constraint = Constraint::Any;
        }
        return i;
    }
    let mut i = unary(Operation::Binary(op), dst, lhs);
    i.input(rhs, Constraint::Any, Phase::Late);
    i
}
fn shift(op: Shift, dst: VirtualReg, lhs: VirtualValue, rhs: VirtualValue) -> Instruction {
    let immediate = match rhs {
        VirtualValue::Imm(n) => Some((n as u8) & 63),
        _ => None,
    };
    let mut i = unary(Operation::Shift(op, immediate), dst, lhs);
    if immediate.is_none() {
        i.input(rhs, Constraint::Fixed(Reg::Rcx), Phase::Late);
    }
    i
}
fn divide(
    signed: bool,
    remainder: bool,
    dst: VirtualReg,
    lhs: VirtualValue,
    rhs: VirtualValue,
) -> Instruction {
    let mut i = Instruction::new(Operation::Divide { signed });
    i.input(lhs, Constraint::Fixed(Reg::Rax), Phase::Early);
    i.input(rhs, Constraint::Any, Phase::Late);
    i.output(
        dst,
        Constraint::Fixed(if remainder { Reg::Rdx } else { Reg::Rax }),
    );
    i.clobbers = vec![Reg::Rax, Reg::Rdx];
    i
}
fn ret(values: &[VirtualValue]) -> Instruction {
    let mut i = Instruction::new(Operation::Return);
    for (&value, reg) in values.iter().zip([Reg::Rax, Reg::Rdx]) {
        i.input(value, Constraint::Fixed(reg), Phase::Early);
    }
    i
}
fn call(
    stream: &mut Vec<Instruction>,
    results: Vec<VirtualReg>,
    target: CallDestination,
    args: Vec<(usize, VirtualValue)>,
) {
    let mut i = Instruction::new(Operation::Call(match &target {
        CallDestination::Direct(symbol) => Some(symbol.clone()),
        _ => None,
    }));
    for (index, value) in args {
        if index < 6 {
            i.input(
                value,
                Constraint::Fixed(Reg::ARGUMENTS[index]),
                Phase::Early,
            );
        } else {
            stream.push(use_one(Operation::Outgoing(index), value));
        }
    }
    if let CallDestination::Indirect(value) = target {
        i.input(value, Constraint::Fixed(Reg::R11), Phase::Early);
    }
    for (value, reg) in results.into_iter().zip([Reg::Rax, Reg::Rdx]) {
        i.output(value, Constraint::Fixed(reg));
    }
    i.clobbers = Reg::CALLER_SAVED.to_vec();
    stream.push(i);
}

// A Phi input from Undefined has no copy, so its value can reach later uses
// with no definition on any path. Such a use reads an arbitrary value: read
// zero instead, and drop Phi copies of undefined sources, so that liveness
// covers only points where a value exists. Dropping a copy can leave another
// value undefined, so repeat until no copy changes.
fn replace_undefined_uses(mut instructions: Vec<Instruction>) -> Vec<Instruction> {
    loop {
        let facts: Vec<_> = instructions.iter().map(Instruction::facts).collect();
        let defined = liveness::DefinedBefore::new(&facts);
        let is_defined = |position: usize, value: VirtualValue| match value {
            VirtualValue::Reg(v) => defined.contains(position, v),
            VirtualValue::Imm(_) => true,
        };
        let mut removed = false;
        for (position, instruction) in instructions.iter_mut().enumerate() {
            for input in &mut instruction.inputs {
                if !is_defined(position, input.value) {
                    input.value = VirtualValue::Imm(0);
                }
            }
            if let Operation::Parallel(copies) = &mut instruction.operation {
                let before = copies.len();
                copies.retain(|&(_, source)| is_defined(position, source));
                removed |= copies.len() != before;
            }
        }
        if !removed {
            return instructions;
        }
    }
}

// Layout canonicalization emits identity copies for 64-bit values. Preserve
// their SSA identity by rewriting uses before liveness, rather than allocating
// two overlapping intervals and paying a machine move for the same value.
fn eliminate_copies(mut instructions: Vec<Instruction>) -> Vec<Instruction> {
    let mut definitions = HashMap::<VirtualReg, usize>::default();
    for instruction in &instructions {
        for v in instruction.facts().definitions {
            *definitions.entry(v).or_default() += 1;
        }
    }
    let aliases: HashMap<_, _> = instructions
        .iter()
        .filter_map(|i| {
            if matches!(i.operation, Operation::Copy) && definitions[&i.outputs[0].value] == 1 {
                Some((i.outputs[0].value, i.inputs[0].value))
            } else {
                None
            }
        })
        .collect();
    let resolve = |mut value: VirtualValue| {
        let mut visited = HashSet::default();
        while let VirtualValue::Reg(v) = value {
            let Some(&next) = aliases.get(&v) else { break };
            if !visited.insert(v) {
                break;
            }
            value = next;
        }
        value
    };
    instructions.retain(|i| {
        !matches!(i.operation, Operation::Copy) || !aliases.contains_key(&i.outputs[0].value)
    });
    for instruction in &mut instructions {
        for input in &mut instruction.inputs {
            input.value = resolve(input.value);
        }
        if let Operation::Parallel(copies) = &mut instruction.operation {
            for (_, src) in copies {
                *src = resolve(*src);
            }
        }
    }
    instructions
}
