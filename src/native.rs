//! i64 native compilation for the sea-of-nodes graph.
//!
//! Lowering follows `NodeId` order, including explicit labels, branches, and loop back
//! edges. Register allocation uses the target-independent linear scanner, with `rax`/`rdx`
//! reserved for division and returns, and `r10`/`r11` for spill scratch.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::regalloc::{
    AllocationResult, LiveInterval, Location, MoveHint, PhysicalRegister, RegAllocError,
    RegisterAllocator, SpillSlot, VirtualReg as AllocVirtualReg,
};
use crate::son::{BinaryOp, CompareOp, Graph, NodeId, NodeKind, ValidationError};
use crate::x86::{
    AsmError, Condition, MachineFunction, MachineInstr, Operand, Reg, StackSlot, VirtualInstr,
    VirtualReg, VirtualValue,
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
pub enum UnsupportedReason {
    NodeKind(NodeKind),
    MultipleStarts,
    MemoryInputs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompileError {
    GraphValidation(ValidationError),
    Unsupported {
        node: NodeId,
        reason: UnsupportedReason,
    },
    MissingReturn,
    CallArity {
        target: String,
        expected: usize,
        actual: usize,
    },
    InvalidValueReference {
        node: NodeId,
        input: NodeId,
    },
    TooManyInstructions,
    TooManyStackSlots,
    MissingAllocation(VirtualReg),
    RegisterAllocation(RegAllocError),
    Assembly(AsmError),
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GraphValidation(error) => write!(formatter, "invalid graph: {error}"),
            Self::Unsupported { node, reason } => match reason {
                UnsupportedReason::NodeKind(kind) => {
                    write!(formatter, "unsupported node {node}: {kind:?}")
                }
                UnsupportedReason::MultipleStarts => {
                    write!(
                        formatter,
                        "unsupported node {node}: multiple function starts"
                    )
                }
                UnsupportedReason::MemoryInputs => {
                    write!(formatter, "unsupported node {node}: memory-state inputs")
                }
            },
            Self::MissingReturn => formatter.write_str("graph has no return"),
            Self::CallArity {
                target,
                expected,
                actual,
            } => write!(
                formatter,
                "function {target} expects {expected} arguments, but {actual} were provided"
            ),
            Self::InvalidValueReference { node, input } => write!(
                formatter,
                "node {node} reads {input}, which is not a previously lowered value"
            ),
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
            Self::Assembly(error) => write!(formatter, "assembly emission failed: {error}"),
        }
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::GraphValidation(error) => Some(error),
            Self::RegisterAllocation(error) => Some(error),
            Self::Assembly(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ValidationError> for CompileError {
    fn from(error: ValidationError) -> Self {
        Self::GraphValidation(error)
    }
}

impl From<RegAllocError> for CompileError {
    fn from(error: RegAllocError) -> Self {
        Self::RegisterAllocation(error)
    }
}

impl From<AsmError> for CompileError {
    fn from(error: AsmError) -> Self {
        Self::Assembly(error)
    }
}

pub fn lower_graph(graph: &Graph) -> Result<Vec<VirtualInstr>, CompileError> {
    graph.validate()?;

    if let Some(second) = graph
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Start)
        .map(|(id, _)| id)
        .nth(1)
    {
        return Err(unsupported(second, UnsupportedReason::MultipleStarts));
    }

    let mut returns = graph
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Return)
        .map(|(id, _)| id);
    if returns.next().is_none() {
        return Err(CompileError::MissingReturn);
    }

    let false_projections = graph
        .iter()
        .filter_map(|(id, node)| match node.kind {
            NodeKind::Projection(1) => Some((node.control[0], id)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let jump_targets = graph
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Region)
        .flat_map(|(region, node)| {
            node.control
                .iter()
                .enumerate()
                .filter(|(_, input)| {
                    graph
                        .node(**input)
                        .is_some_and(|node| node.kind == NodeKind::Jump)
                })
                .map(move |(index, input)| (*input, (region, index)))
                .collect::<Vec<_>>()
        })
        .collect::<BTreeMap<_, _>>();
    let loops = loop_headers(graph);
    let direct_comparisons = direct_branch_comparisons(graph);
    let immediate_constants = immediate_constants(graph);
    let mut next_virtual_reg = graph
        .ids()
        .map(|id| id.0)
        .max()
        .and_then(|id| id.checked_add(1))
        .unwrap_or(0);

    let mut instructions = Vec::new();
    let mut values = BTreeSet::new();

    for (id, node) in graph.iter() {
        if !node.memory.is_empty() {
            return Err(unsupported(id, UnsupportedReason::MemoryInputs));
        }
        if node.kind == NodeKind::Start {
            continue;
        }

        let instruction = match node.kind {
            NodeKind::Start => unreachable!(),
            NodeKind::Const(_) if immediate_constants.contains(&id) => VirtualInstr::Nop,
            NodeKind::Const(value) => VirtualInstr::Mov {
                dst: virtual_reg(id),
                src: VirtualValue::Imm(value),
            },
            NodeKind::Parameter { index } => VirtualInstr::LoadArg {
                dst: virtual_reg(id),
                index,
            },
            NodeKind::Unary(operation) => {
                let src = value_reference(id, node.data[0], &values)?;
                match operation {
                    crate::son::UnaryOp::Neg => VirtualInstr::Neg {
                        dst: virtual_reg(id),
                        src,
                    },
                    crate::son::UnaryOp::Not => VirtualInstr::Not {
                        dst: virtual_reg(id),
                        src,
                    },
                }
            }
            NodeKind::Binary(operation) => {
                let (lhs, rhs) = if matches!(
                    operation,
                    BinaryOp::Add | BinaryOp::Mul | BinaryOp::And | BinaryOp::Or | BinaryOp::Xor
                ) && matches!(
                    graph.node(node.data[0]).map(|node| node.kind),
                    Some(NodeKind::Const(_))
                ) && !matches!(
                    graph.node(node.data[1]).map(|node| node.kind),
                    Some(NodeKind::Const(_))
                ) {
                    let lhs = value_reference(id, node.data[1], &values)?;
                    let Some(NodeKind::Const(value)) =
                        graph.node(node.data[0]).map(|node| node.kind)
                    else {
                        unreachable!("left constant was checked")
                    };
                    (lhs, VirtualValue::Imm(value))
                } else {
                    binary_inputs(id, &node.data, &values)?
                };
                let rhs = if let Some(NodeKind::Const(value)) =
                    graph.node(node.data[1]).map(|node| node.kind)
                {
                    VirtualValue::Imm(value)
                } else {
                    rhs
                };
                match operation {
                    BinaryOp::Add => VirtualInstr::Add {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::Sub => VirtualInstr::Sub {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::Mul => VirtualInstr::Mul {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::Div => VirtualInstr::Div {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::Rem => VirtualInstr::Rem {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::UDiv => VirtualInstr::UDiv {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::URem => VirtualInstr::URem {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::And => VirtualInstr::And {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::Or => VirtualInstr::Or {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::Xor => VirtualInstr::Xor {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::Shl => VirtualInstr::Shl {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::Shr => VirtualInstr::Shr {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                    BinaryOp::LShr => VirtualInstr::LShr {
                        dst: virtual_reg(id),
                        lhs,
                        rhs,
                    },
                }
            }
            NodeKind::Compare(_) if direct_comparisons.contains_key(&id) => VirtualInstr::Nop,
            NodeKind::Compare(operation) => {
                let (lhs, rhs) = binary_inputs(id, &node.data, &values)?;
                let rhs = if let Some(NodeKind::Const(value)) =
                    graph.node(node.data[1]).map(|node| node.kind)
                {
                    VirtualValue::Imm(value)
                } else {
                    rhs
                };
                VirtualInstr::Compare {
                    dst: virtual_reg(id),
                    condition: condition(operation),
                    lhs,
                    rhs,
                }
            }
            NodeKind::Call(target) => {
                let metadata = graph
                    .functions
                    .get(target)
                    .expect("frontend validated the call target");
                if metadata.parameter_count != node.data.len() {
                    return Err(CompileError::CallArity {
                        target: metadata.name.clone(),
                        expected: metadata.parameter_count,
                        actual: node.data.len(),
                    });
                }
                let target = metadata.name.clone();
                let args = node
                    .data
                    .iter()
                    .map(|input| value_reference(id, *input, &values))
                    .collect::<Result<_, _>>()?;
                VirtualInstr::Call {
                    dst: virtual_reg(id),
                    target,
                    args,
                }
            }
            NodeKind::If => {
                let condition_id = node.data[0];
                let target = false_projections
                    .get(&id)
                    .ok_or_else(|| unsupported(id, UnsupportedReason::NodeKind(node.kind)))?;
                let (condition, lhs, rhs) =
                    if let Some(inverse) = direct_comparisons.get(&condition_id) {
                        let compare = graph.node(condition_id).expect("direct comparison exists");
                        let (lhs, rhs) = binary_inputs(id, &compare.data, &values)?;
                        let rhs = if let Some(NodeKind::Const(value)) =
                            graph.node(compare.data[1]).map(|node| node.kind)
                        {
                            VirtualValue::Imm(value)
                        } else {
                            rhs
                        };
                        (*inverse, lhs, rhs)
                    } else {
                        (
                            Condition::Eq,
                            value_reference(id, condition_id, &values)?,
                            VirtualValue::Imm(0),
                        )
                    };
                instructions.push(VirtualInstr::Test { lhs, rhs });
                VirtualInstr::Jcc {
                    condition,
                    target: target.0,
                }
            }
            NodeKind::Jump => {
                let (region, input_index) = jump_targets
                    .get(&id)
                    .copied()
                    .ok_or_else(|| unsupported(id, UnsupportedReason::NodeKind(node.kind)))?;
                emit_phi_edge(
                    &mut instructions,
                    id,
                    graph,
                    region,
                    input_index,
                    &values,
                    &mut next_virtual_reg,
                )?;
                VirtualInstr::Jmp(region.0)
            }
            NodeKind::Projection(_) => VirtualInstr::Label(id.0),
            NodeKind::Region if loops.contains_key(&id) => {
                emit_phi_edge(
                    &mut instructions,
                    id,
                    graph,
                    id,
                    0,
                    &values,
                    &mut next_virtual_reg,
                )?;
                VirtualInstr::Label(id.0)
            }
            NodeKind::Region => VirtualInstr::Label(id.0),
            NodeKind::Phi => VirtualInstr::Phi {
                dst: virtual_reg(id),
            },
            NodeKind::Return => match node.data.first() {
                Some(&value) => VirtualInstr::Return(value_reference(id, value, &values)?),
                None => VirtualInstr::Return(VirtualValue::Imm(0)),
            },
        };

        if matches!(node.kind, NodeKind::Return) {
            instructions.push(instruction);
        } else {
            values.insert(id);
            instructions.push(instruction);
        }
    }

    Ok(instructions)
}

fn loop_headers(graph: &Graph) -> BTreeMap<NodeId, NodeId> {
    graph
        .iter()
        .filter_map(|(header, node)| {
            if node.kind != NodeKind::Region || node.control.len() < 2 {
                return None;
            }
            let back_edge = node.control.iter().skip(1).find(|edge| {
                **edge > header
                    && graph
                        .node(**edge)
                        .is_some_and(|node| node.kind == NodeKind::Jump)
            })?;
            Some((header, *back_edge))
        })
        .collect()
}

fn emit_phi_edge(
    instructions: &mut Vec<VirtualInstr>,
    node: NodeId,
    graph: &Graph,
    header: NodeId,
    input_index: usize,
    values: &BTreeSet<NodeId>,
    next_virtual_reg: &mut u32,
) -> Result<(), CompileError> {
    let controls = &graph.node(header).expect("loop header exists").control;
    let mut moves = Vec::new();
    for (phi, phi_node) in graph.iter().filter(|(_, node)| node.kind == NodeKind::Phi) {
        if phi_node.control == *controls {
            let src = value_reference(node, phi_node.data[input_index], values)?;
            if VirtualValue::Reg(virtual_reg(phi)) != src {
                moves.push((phi, src));
            }
        }
    }
    let destinations = moves
        .iter()
        .map(|(phi, _)| virtual_reg(*phi))
        .collect::<Vec<_>>();
    let has_cycle = moves.iter().any(|(_, src)| match src {
        VirtualValue::Reg(src) => destinations.contains(src),
        VirtualValue::Imm(_) => false,
    });
    if !has_cycle {
        for (phi, src) in moves {
            instructions.push(VirtualInstr::Mov {
                dst: virtual_reg(phi),
                src,
            });
        }
        return Ok(());
    }

    let mut temporaries = Vec::new();
    for (phi, src) in moves {
        let temporary = VirtualReg(*next_virtual_reg);
        *next_virtual_reg = next_virtual_reg
            .checked_add(1)
            .ok_or(CompileError::TooManyInstructions)?;
        instructions.push(VirtualInstr::Mov {
            dst: temporary,
            src,
        });
        temporaries.push((phi, temporary));
    }
    for (phi, temporary) in temporaries {
        instructions.push(VirtualInstr::Mov {
            dst: virtual_reg(phi),
            src: VirtualValue::Reg(temporary),
        });
    }
    Ok(())
}

pub fn compile_graph(
    graph: &Graph,
    name: impl Into<String>,
) -> Result<MachineFunction, CompileError> {
    let virtual_instructions = lower_graph(graph)?;
    let intervals = live_intervals(&virtual_instructions)?;
    let move_hints = collect_move_hints(&virtual_instructions)?;
    let allocator = RegisterAllocator::new(GENERAL_REGISTERS.len() as u16);
    let allocation = allocator.allocate_with_move_hints(&intervals, &move_hints)?;
    let argument_homes = required_argument_homes(&virtual_instructions, &allocation);
    let spill_slot_offset = argument_homes.len() as u32;
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
        symbols: &function.symbols,
        instructions: Vec::with_capacity(virtual_instructions.len()),
    };
    for instruction in &virtual_instructions {
        emitter.emit(instruction.clone())?;
    }
    function.instructions.extend(emitter.instructions);
    Ok(function)
}

pub fn compile_graph_to_assembly(
    graph: &Graph,
    name: impl Into<String>,
) -> Result<String, CompileError> {
    Ok(compile_graph(graph, name)?.emit_gnu_assembly()?)
}

fn unsupported(node: NodeId, reason: UnsupportedReason) -> CompileError {
    CompileError::Unsupported { node, reason }
}

fn virtual_reg(node: NodeId) -> VirtualReg {
    VirtualReg(node.0)
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

fn value_reference(
    node: NodeId,
    input: NodeId,
    values: &BTreeSet<NodeId>,
) -> Result<VirtualValue, CompileError> {
    if values.contains(&input) {
        Ok(VirtualValue::Reg(virtual_reg(input)))
    } else {
        Err(CompileError::InvalidValueReference { node, input })
    }
}

fn binary_inputs(
    node: NodeId,
    inputs: &[NodeId],
    values: &BTreeSet<NodeId>,
) -> Result<(VirtualValue, VirtualValue), CompileError> {
    debug_assert_eq!(inputs.len(), 2, "graph validation checked binary arity");
    Ok((
        value_reference(node, inputs[0], values)?,
        value_reference(node, inputs[1], values)?,
    ))
}

fn condition(operation: CompareOp) -> Condition {
    match operation {
        CompareOp::Eq => Condition::Eq,
        CompareOp::Ne => Condition::NotEq,
        CompareOp::Lt => Condition::SignedLess,
        CompareOp::Le => Condition::SignedLessEq,
        CompareOp::Gt => Condition::SignedGreater,
        CompareOp::Ge => Condition::SignedGreaterEq,
        CompareOp::ULt => Condition::UnsignedBelow,
        CompareOp::ULe => Condition::UnsignedBelowEq,
        CompareOp::UGt => Condition::UnsignedAbove,
        CompareOp::UGe => Condition::UnsignedAboveEq,
    }
}

fn direct_branch_comparisons(graph: &Graph) -> BTreeMap<NodeId, Condition> {
    let mut candidates = BTreeMap::new();
    let mut rejected = BTreeSet::new();
    for (_, node) in graph.iter() {
        for channel in [
            crate::son::InputChannel::Control,
            crate::son::InputChannel::Data,
            crate::son::InputChannel::Memory,
        ] {
            for input in node.inputs(channel) {
                let Some(NodeKind::Compare(operation)) = graph.node(*input).map(|node| node.kind)
                else {
                    continue;
                };
                if node.kind == NodeKind::If && node.data[0] == *input {
                    candidates.insert(*input, invert_condition(condition(operation)));
                } else {
                    rejected.insert(*input);
                }
            }
        }
    }
    candidates.retain(|id, _| !rejected.contains(id));
    candidates
}

fn immediate_constants(graph: &Graph) -> BTreeSet<NodeId> {
    let mut candidates = BTreeSet::new();
    let mut rejected = BTreeSet::new();
    for (_, node) in graph.iter() {
        for channel in [
            crate::son::InputChannel::Control,
            crate::son::InputChannel::Memory,
        ] {
            for input in node.inputs(channel) {
                rejected.insert(*input);
            }
        }
        for (index, input) in node.data.iter().enumerate() {
            if (index == 1 && matches!(node.kind, NodeKind::Binary(_) | NodeKind::Compare(_)))
                || (index == 0
                    && matches!(
                        node.kind,
                        NodeKind::Binary(
                            BinaryOp::Add
                                | BinaryOp::Mul
                                | BinaryOp::And
                                | BinaryOp::Or
                                | BinaryOp::Xor
                        )
                    )
                    && !matches!(
                        graph.node(node.data[1]).map(|node| node.kind),
                        Some(NodeKind::Const(_))
                    ))
            {
                candidates.insert(*input);
            } else {
                rejected.insert(*input);
            }
        }
    }
    candidates.retain(|id| !rejected.contains(id));
    candidates
}

fn invert_condition(condition: Condition) -> Condition {
    match condition {
        Condition::Eq => Condition::NotEq,
        Condition::NotEq => Condition::Eq,
        Condition::SignedLess => Condition::SignedGreaterEq,
        Condition::SignedLessEq => Condition::SignedGreater,
        Condition::SignedGreater => Condition::SignedLessEq,
        Condition::SignedGreaterEq => Condition::SignedLess,
        Condition::UnsignedBelow => Condition::UnsignedAboveEq,
        Condition::UnsignedBelowEq => Condition::UnsignedAbove,
        Condition::UnsignedAbove => Condition::UnsignedBelowEq,
        Condition::UnsignedAboveEq => Condition::UnsignedBelow,
    }
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

fn live_intervals(instructions: &[VirtualInstr]) -> Result<Vec<LiveInterval>, CompileError> {
    let mut intervals: BTreeMap<u32, (u32, u32)> = BTreeMap::new();

    for (index, instruction) in instructions.iter().enumerate() {
        let position = u32::try_from(index).map_err(|_| CompileError::TooManyInstructions)?;
        let end = position
            .checked_add(1)
            .ok_or(CompileError::TooManyInstructions)?;

        if let Some(definition) = defined_register(instruction) {
            intervals
                .entry(definition.0)
                .and_modify(|(_, interval_end)| *interval_end = end)
                .or_insert((position, end));
        }
        for used in used_registers(instruction) {
            if let Some((_, interval_end)) = intervals.get_mut(&used.0) {
                *interval_end = end;
            }
        }
    }

    let labels = instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| match instruction {
            VirtualInstr::Label(label) => Some((*label, index)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let loop_ranges = instructions
        .iter()
        .enumerate()
        .filter_map(|(back, instruction)| match instruction {
            VirtualInstr::Jmp(target) => labels
                .get(target)
                .filter(|header| **header < back)
                .map(|header| (*header, back)),
            _ => None,
        })
        .map(|(header, back)| (header as u32, back as u32))
        .collect::<Vec<_>>();
    for (start, end) in intervals.values_mut() {
        for (header, back) in &loop_ranges {
            if *start < *header && *end > *header && *end <= *back {
                *end = *back + 1;
            }
        }
    }

    Ok(intervals
        .into_iter()
        .map(|(virtual_reg, (start, end))| {
            LiveInterval::new(AllocVirtualReg(virtual_reg), start, end)
        })
        .collect())
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
                        slot: StackSlot(self.spill_slot_offset + CALL_HOME_COUNT + index as u32),
                        src: SCRATCH1,
                    });
                }
                for index in 0..args.len() {
                    let slot = StackSlot(self.spill_slot_offset + CALL_HOME_COUNT + index as u32);
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
                for (index, register) in CALL_HOME_REGISTERS.iter().copied().enumerate() {
                    self.instructions.push(MachineInstr::Store {
                        slot: StackSlot(self.spill_slot_offset + index as u32),
                        src: register,
                    });
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
                        slot: StackSlot(self.spill_slot_offset + index as u32),
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
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::machine_eval::{self, MachineEvalError};
    use crate::son::Node;

    fn graph_with_return(value: NodeId, start: NodeId, graph: &mut Graph) {
        graph.return_(start, Some(value));
    }

    #[test]
    fn compiles_constants_arithmetic_and_gnu_assembly() {
        let mut graph = Graph::new();
        let start = graph.start();
        let seven = graph.constant(7);
        let three = graph.constant(3);
        let difference = graph.binary(BinaryOp::Sub, seven, three);
        let two = graph.constant(2);
        let product = graph.binary(BinaryOp::Mul, difference, two);
        let five = graph.constant(5);
        let sum = graph.binary(BinaryOp::Add, product, five);
        graph_with_return(sum, start, &mut graph);

        let function = compile_graph(&graph, "arithmetic").unwrap();
        assert_eq!(run(&function, &[]), 13);
        assert_eq!(function.stack_slots, 0);
        assert!(function.callee_saved.is_empty());

        let assembly = compile_graph_to_assembly(&graph, "arithmetic").unwrap();
        assert!(assembly.starts_with(".intel_syntax noprefix\n.text\n.globl arithmetic\n"));
        assert!(assembly.contains("\timul"));
        assert!(assembly.ends_with(
            ".size arithmetic, .-arithmetic\n.section .note.GNU-stack,\"\",@progbits\n"
        ));
    }

    #[test]
    fn signed_division_and_remainder_use_rax_rdx_in_lowering_order() {
        assert!(!GENERAL_REGISTERS.contains(&Reg::Rax));
        assert!(!GENERAL_REGISTERS.contains(&Reg::Rdx));

        for (operation, lhs, rhs, expected, result_register) in [
            (BinaryOp::Div, -42, 4, -10, Reg::Rax),
            (BinaryOp::Rem, -42, 4, -2, Reg::Rdx),
            (BinaryOp::Div, 42, -4, -10, Reg::Rax),
            (BinaryOp::Rem, 42, -4, 2, Reg::Rdx),
            (BinaryOp::Div, i64::MIN, 1, i64::MIN, Reg::Rax),
            (BinaryOp::Rem, i64::MIN, 1, 0, Reg::Rdx),
        ] {
            let mut graph = Graph::new();
            let start = graph.start();
            let lhs_node = graph.parameter(0);
            let rhs_node = graph.parameter(1);
            let result = graph.binary(operation, lhs_node, rhs_node);
            graph_with_return(result, start, &mut graph);

            let function = compile_graph(&graph, "signed_division").unwrap();
            assert_eq!(run(&function, &[lhs, rhs]), expected);
            assert_eq!(machine_eval::evaluate(&function, &[lhs, rhs]), Ok(expected));
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
        for (operation, expected) in [(BinaryOp::Div, 11), (BinaryOp::Rem, 18)] {
            let mut graph = Graph::new();
            let start = graph.start();
            let lhs = graph.parameter(0);
            let rhs = graph.parameter(1);
            let division = graph.binary(operation, lhs, rhs);
            let third = graph.parameter(2);
            let seventh = graph.parameter(6);
            let product = graph.binary(BinaryOp::Mul, third, seventh);
            let result = graph.binary(BinaryOp::Add, division, product);
            graph_with_return(result, start, &mut graph);

            let arguments = [-43, 4, 7, 0, 0, 0, 3];
            let function = compile_graph(&graph, "division_arguments").unwrap();
            assert!(matches!(
                function.instructions.first(),
                Some(MachineInstr::Store { src: Reg::Rdx, .. })
            ));
            assert_eq!(run(&function, &arguments), expected);
            assert_eq!(machine_eval::evaluate(&function, &arguments), Ok(expected));
        }
    }

    #[test]
    fn division_and_remainder_accept_spilled_divisors() {
        for (operation, expected) in [(BinaryOp::Div, 207), (BinaryOp::Rem, 208)] {
            let mut graph = Graph::new();
            let start = graph.start();
            let mut values = Vec::new();
            for value in 1..=20 {
                values.push(graph.parameter((value - 1) as usize));
            }
            let mut sum = values[2];
            for value in values.iter().skip(3) {
                sum = graph.binary(BinaryOp::Add, sum, *value);
            }
            let division = graph.binary(operation, values[0], values[1]);
            let result = graph.binary(BinaryOp::Add, sum, division);
            graph_with_return(result, start, &mut graph);

            let function = compile_graph(&graph, "spilled_divisor").unwrap();
            assert!(function.stack_slots > 0);
            assert!(function.instructions.iter().any(|instruction| matches!(
                instruction,
                MachineInstr::IDiv {
                    src: Operand::Stack(_)
                }
            )));
            assert_eq!(run(&function, &sequence(1, 20)), expected);
            assert_eq!(
                machine_eval::evaluate(&function, &sequence(1, 20)),
                Ok(expected)
            );
        }
    }

    #[test]
    fn division_and_remainder_spill_results_without_losing_them() {
        for (operation, expected) in [(BinaryOp::Div, 200), (BinaryOp::Rem, 207)] {
            let mut graph = Graph::new();
            let start = graph.start();
            let lhs = graph.constant(-43);
            let rhs = graph.constant(4);
            let division = graph.binary(operation, lhs, rhs);
            let mut values = Vec::new();
            for value in 1..=20 {
                values.push(graph.parameter((value - 1) as usize));
            }
            let mut sum = values[0];
            for value in values.into_iter().skip(1) {
                sum = graph.binary(BinaryOp::Add, sum, value);
            }
            let result = graph.binary(BinaryOp::Add, sum, division);
            graph_with_return(result, start, &mut graph);

            let function = compile_graph(&graph, "spilled_division_result").unwrap();
            let result_position = match operation {
                BinaryOp::Div => {
                    let shift = function
                        .instructions
                        .iter()
                        .position(|instruction| {
                            matches!(instruction, MachineInstr::ShrImm { count: 2, .. })
                        })
                        .unwrap();
                    shift + 1
                }
                BinaryOp::Rem => {
                    function
                        .instructions
                        .iter()
                        .position(|instruction| matches!(instruction, MachineInstr::IDiv { .. }))
                        .unwrap()
                        + 2
                }
                _ => unreachable!(),
            };
            assert!(matches!(
                function.instructions[result_position],
                MachineInstr::Store { .. }
            ));
            assert_eq!(run(&function, &sequence(1, 20)), expected);
            assert_eq!(
                machine_eval::evaluate(&function, &sequence(1, 20)),
                Ok(expected)
            );
        }
    }

    #[test]
    fn unused_trapping_division_and_remainder_are_not_discarded() {
        for (operation, lhs, rhs, expected) in [
            (BinaryOp::Div, 1, 0, MachineEvalError::DivisionByZero),
            (BinaryOp::Rem, 1, 0, MachineEvalError::DivisionByZero),
            (
                BinaryOp::Div,
                i64::MIN,
                -1,
                MachineEvalError::DivisionOverflow,
            ),
            (
                BinaryOp::Rem,
                i64::MIN,
                -1,
                MachineEvalError::DivisionOverflow,
            ),
        ] {
            let mut graph = Graph::new();
            let start = graph.start();
            let lhs = graph.constant(lhs);
            let rhs = graph.constant(rhs);
            graph.binary(operation, lhs, rhs);
            let one = graph.constant(1);
            graph_with_return(one, start, &mut graph);

            let function = compile_graph(&graph, "trapping_division").unwrap();
            assert!(
                function
                    .instructions
                    .iter()
                    .any(|instruction| matches!(instruction, MachineInstr::IDiv { .. }))
            );
            assert_eq!(machine_eval::evaluate(&function, &[]), Err(expected));
        }
    }

    #[test]
    fn supports_register_stack_and_repeated_parameters() {
        let mut graph = Graph::new();
        let start = graph.start();
        let first = graph.parameter(0);
        let second = graph.parameter(1);
        let seventh = graph.parameter(6);
        let sum = graph.binary(BinaryOp::Add, first, second);
        let product = graph.binary(BinaryOp::Mul, sum, seventh);
        let repeated = graph.parameter(0);
        let result = graph.binary(BinaryOp::Add, product, repeated);
        graph_with_return(result, start, &mut graph);

        let args = [9, 4, 0, 0, 0, 0, 3];
        assert_eq!(run(&compile_graph(&graph, "params").unwrap(), &args), 48);

        let assembly = compile_graph_to_assembly(&graph, "params").unwrap();
        assert!(assembly.contains("QWORD PTR [rbp+16]"), "{assembly}");
    }

    #[test]
    fn parameter_homes_preserve_clobbered_incoming_registers() {
        let mut graph = Graph::new();
        let start = graph.start();
        let first = graph.parameter(0);
        let mut values = Vec::new();
        for value in 0..12 {
            values.push(graph.parameter(value as usize));
        }
        let second = graph.parameter(1);
        let mut sum = graph.binary(BinaryOp::Add, first, second);
        for value in values {
            sum = graph.binary(BinaryOp::Add, sum, value);
        }
        graph_with_return(sum, start, &mut graph);

        let function = compile_graph(&graph, "argument_home").unwrap();
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
        let mut graph = Graph::new();
        let start = graph.start();
        let mut values = Vec::new();
        for value in 1..=20 {
            values.push(graph.parameter((value - 1) as usize));
        }
        let mut sum = values[0];
        for value in values.into_iter().skip(1) {
            sum = graph.binary(BinaryOp::Add, sum, value);
        }
        graph_with_return(sum, start, &mut graph);

        let function = compile_graph(&graph, "pressure").unwrap();
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
            (CompareOp::Eq, [0, 1, 0]),
            (CompareOp::Ne, [1, 0, 1]),
            (CompareOp::Lt, [1, 0, 0]),
            (CompareOp::Le, [1, 1, 0]),
            (CompareOp::Gt, [0, 0, 1]),
            (CompareOp::Ge, [0, 1, 1]),
        ];

        for (operation, expected) in cases {
            let mut graph = Graph::new();
            let start = graph.start();
            let lhs = graph.parameter(0);
            let rhs = graph.parameter(1);
            let comparison = graph.compare(operation, lhs, rhs);
            graph_with_return(comparison, start, &mut graph);
            let function = compile_graph(&graph, "compare").unwrap();
            for ((lhs, rhs), expected) in [(4, 5), (5, 5), (6, 5)].into_iter().zip(expected) {
                assert_eq!(run(&function, &[lhs, rhs]), expected, "{operation:?}");
            }
        }
    }

    #[test]
    fn spilled_compare_uses_scratch_and_stores_boolean() {
        let mut graph = Graph::new();
        let start = graph.start();
        let mut values = Vec::new();
        for value in 0..18 {
            values.push(graph.parameter(value as usize));
        }
        let comparison = graph.compare(CompareOp::Lt, values[0], values[1]);
        let mut sum = values[2];
        for value in values.into_iter().skip(3) {
            sum = graph.binary(BinaryOp::Add, sum, value);
        }
        let result = graph.binary(BinaryOp::Add, sum, comparison);
        graph_with_return(result, start, &mut graph);

        let function = compile_graph(&graph, "spilled_compare").unwrap();
        assert!(function.instructions.iter().any(|instruction| {
            matches!(instruction, MachineInstr::SetCc { dst: SCRATCH0, .. })
        }));
        assert_eq!(run(&function, &sequence(0, 18)), 153);
    }

    #[test]
    fn subtraction_keeps_lhs_and_rhs_order_even_with_spills() {
        let mut graph = Graph::new();
        let start = graph.start();
        let mut values = Vec::new();
        for value in 0..16 {
            values.push(graph.parameter(value as usize));
        }
        let difference = graph.binary(BinaryOp::Sub, values[0], values[15]);
        let mut sum = difference;
        for value in values.into_iter().skip(1).take(14) {
            sum = graph.binary(BinaryOp::Add, sum, value);
        }
        graph_with_return(sum, start, &mut graph);

        let function = compile_graph(&graph, "sub_spills").unwrap();
        assert!(function.stack_slots > 0);
        assert_eq!(run(&function, &sequence(0, 16)), 90);
    }

    #[test]
    fn supports_multiple_returns() {
        let mut graph = Graph::new();
        let start = graph.start();
        let one = graph.constant(1);
        graph.return_(start, Some(one));
        graph.return_(start, Some(one));
        let function = compile_graph(&graph, "returns").unwrap();
        assert_eq!(run(&function, &[]), 1);
        assert_eq!(
            function
                .instructions
                .iter()
                .filter(|instruction| matches!(instruction, MachineInstr::Ret))
                .count(),
            2
        );

        let mut graph = Graph::new();
        let start = graph.start();
        let one = graph.constant(1);
        let phi = graph.phi(vec![start], vec![one]);
        graph.return_(start, Some(phi));
        assert!(lower_graph(&graph).is_ok());
    }

    #[test]
    fn rejects_missing_empty_and_early_references() {
        let mut graph = Graph::new();
        graph.constant(1);
        assert_eq!(lower_graph(&graph), Err(CompileError::MissingReturn));

        let mut graph = Graph::new();
        let start = graph.start();
        graph.return_(start, None);
        assert_eq!(
            lower_graph(&graph).unwrap().as_slice(),
            &[VirtualInstr::Return(VirtualValue::Imm(0))]
        );

        let mut graph = Graph::new();
        let start = graph.start();
        let addition = graph.push(Node::with_inputs(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![NodeId(3), NodeId(3)],
        ));
        graph.constant(1);
        graph.return_(start, Some(addition));
        assert_eq!(
            lower_graph(&graph),
            Err(CompileError::InvalidValueReference {
                node: addition,
                input: NodeId(3),
            })
        );
    }

    fn sequence(start: i64, count: usize) -> Vec<i64> {
        (start..start + count as i64).collect()
    }

    fn run(function: &MachineFunction, args: &[i64]) -> i64 {
        let mut registers = HashMap::new();
        for (index, register) in Reg::ARGUMENTS.into_iter().enumerate() {
            registers.insert(register, args.get(index).copied().unwrap_or(0));
        }
        let mut slots = vec![0; function.stack_slots as usize];
        let mut comparison = None;

        for instruction in &function.instructions {
            match *instruction {
                MachineInstr::LoadArg { dst, index } => {
                    let value = if index < Reg::ARGUMENTS.len() {
                        registers.get(&Reg::ARGUMENTS[index]).copied().unwrap_or(0)
                    } else {
                        args[index]
                    };
                    registers.insert(dst, value);
                }
                MachineInstr::MovImm { dst, value } => {
                    registers.insert(dst, value);
                }
                MachineInstr::Mov { dst, src } => {
                    registers.insert(dst, read_reg(&registers, src));
                }
                MachineInstr::Neg { dst } => {
                    let value = read_reg(&registers, dst).wrapping_neg();
                    registers.insert(dst, value);
                }
                MachineInstr::Not { dst } => {
                    let value = !read_reg(&registers, dst);
                    registers.insert(dst, value);
                }
                MachineInstr::Add { dst, src } => {
                    let value = read_reg(&registers, dst)
                        .wrapping_add(read_operand(&registers, &slots, src));
                    registers.insert(dst, value);
                }
                MachineInstr::Sub { dst, src } => {
                    let value = read_reg(&registers, dst)
                        .wrapping_sub(read_operand(&registers, &slots, src));
                    registers.insert(dst, value);
                }
                MachineInstr::IMul { dst, src } => {
                    let value = read_reg(&registers, dst)
                        .wrapping_mul(read_operand(&registers, &slots, src));
                    registers.insert(dst, value);
                }
                MachineInstr::And { dst, src } => {
                    let value = read_reg(&registers, dst) & read_operand(&registers, &slots, src);
                    registers.insert(dst, value);
                }
                MachineInstr::Or { dst, src } => {
                    let value = read_reg(&registers, dst) | read_operand(&registers, &slots, src);
                    registers.insert(dst, value);
                }
                MachineInstr::Xor { dst, src } => {
                    let value = read_reg(&registers, dst) ^ read_operand(&registers, &slots, src);
                    registers.insert(dst, value);
                }
                MachineInstr::Shl { dst } => {
                    let value =
                        read_reg(&registers, dst)
                            .wrapping_shl(read_reg(&registers, Reg::Rcx) as u32);
                    registers.insert(dst, value);
                }
                MachineInstr::Shr { dst } => {
                    let value =
                        read_reg(&registers, dst)
                            .wrapping_shr(read_reg(&registers, Reg::Rcx) as u32);
                    registers.insert(dst, value);
                }
                MachineInstr::LShr { dst } => {
                    let value = (read_reg(&registers, dst) as u64).wrapping_shr(read_reg(
                        &registers,
                        Reg::Rcx,
                    )
                        as u32);
                    registers.insert(dst, value as i64);
                }
                MachineInstr::ShlImm { dst, count } => {
                    let value = read_reg(&registers, dst).wrapping_shl(count as u32);
                    registers.insert(dst, value);
                }
                MachineInstr::ShrImm { dst, count } => {
                    let value = read_reg(&registers, dst).wrapping_shr(count as u32);
                    registers.insert(dst, value);
                }
                MachineInstr::LShrImm { dst, count } => {
                    let value = (read_reg(&registers, dst) as u64).wrapping_shr(count as u32);
                    registers.insert(dst, value as i64);
                }
                MachineInstr::Cqo => {
                    let sign = if read_reg(&registers, Reg::Rax) < 0 {
                        -1
                    } else {
                        0
                    };
                    registers.insert(Reg::Rdx, sign);
                }
                MachineInstr::IDiv { src } => {
                    let lhs = read_reg(&registers, Reg::Rax);
                    let rhs = read_operand(&registers, &slots, src);
                    let quotient = lhs.checked_div(rhs).expect("division must not trap");
                    let remainder = lhs.checked_rem(rhs).expect("remainder must not trap");
                    registers.insert(Reg::Rax, quotient);
                    registers.insert(Reg::Rdx, remainder);
                }
                MachineInstr::UDiv { src } => {
                    let lhs = read_reg(&registers, Reg::Rax) as u64;
                    let rhs = read_operand(&registers, &slots, src) as u64;
                    let quotient = lhs.checked_div(rhs).expect("division must not trap");
                    let remainder = lhs.checked_rem(rhs).expect("remainder must not trap");
                    registers.insert(Reg::Rax, quotient as i64);
                    registers.insert(Reg::Rdx, remainder as i64);
                }
                MachineInstr::Cmp { lhs, rhs } => {
                    comparison = Some((
                        read_reg(&registers, lhs),
                        read_operand(&registers, &slots, rhs),
                    ));
                }
                MachineInstr::Test { reg } => {
                    comparison = Some((read_reg(&registers, reg), 0));
                }
                MachineInstr::SetCc { condition, dst } => {
                    let (lhs, rhs) = comparison.expect("compare before setcc");
                    let value = match condition {
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
                    };
                    registers.insert(dst, i64::from(value));
                }
                MachineInstr::Load { dst, slot } => {
                    registers.insert(dst, slots[slot.0 as usize]);
                }
                MachineInstr::Store { slot, src } => {
                    slots[slot.0 as usize] = read_reg(&registers, src);
                }
                MachineInstr::StoreArg { .. } => {
                    unreachable!("the simple native test evaluator does not model calls")
                }
                MachineInstr::Label(_) | MachineInstr::Jmp(_) | MachineInstr::Jcc { .. } => {
                    unreachable!("the simple native test evaluator does not model control flow")
                }
                MachineInstr::Call { .. } => {
                    unreachable!("the simple native test evaluator does not model calls")
                }
                MachineInstr::Ret => return read_reg(&registers, Reg::RETURN),
            }
        }
        panic!("machine function did not return")
    }

    fn read_reg(registers: &HashMap<Reg, i64>, register: Reg) -> i64 {
        registers.get(&register).copied().unwrap_or(0)
    }

    fn read_operand(registers: &HashMap<Reg, i64>, slots: &[i64], operand: Operand) -> i64 {
        match operand {
            Operand::Reg(register) => read_reg(registers, register),
            Operand::Stack(slot) => slots[slot.0 as usize],
            Operand::Imm(value) => value as i64,
        }
    }
}
