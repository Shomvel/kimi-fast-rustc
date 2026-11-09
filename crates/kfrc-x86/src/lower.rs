//! Graph → `VirtualInstr` → `MachineFunction`.

mod abi;
mod cast;
mod fallthrough;
mod memory;
mod memory_liveness;
mod schedule;
mod vector;

use crate::analysis::liveness::live_intervals;
use memory::compile_memory_function;
use schedule::Schedule;

use std::collections::{HashMap, HashSet};

use crate::machine::allocate::{CompileError, compile_virtual_function};
use crate::machine::x86::{
    Condition, MachineFunction, MachineInstr, Operand as MachineOperand, Reg, StackSlot,
    VirtualInstr, VirtualReg, VirtualValue, Xmm,
};
use kfrc_body::{PassMode, SizeAlign, TyId, TypeOracle, TypeShape};
use kfrc_son::lower_layout::LayoutOptions;
use kfrc_son::{
    ComparePredicate, FunctionGraph, GraphLevel, GraphState, NodeId, Opcode, ValueType,
};

#[derive(Debug)]
pub enum LowerError {
    NotVerified,
    Unsupported(String),
    Malformed(String),
    Backend(CompileError),
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotVerified => formatter.write_str("graph must be verified before lowering"),
            Self::Unsupported(what) => write!(formatter, "unsupported in x86 lowering: {what}"),
            Self::Malformed(what) => write!(formatter, "malformed graph: {what}"),
            Self::Backend(error) => {
                write!(formatter, "register allocation or emission failed: {error}")
            }
        }
    }
}

impl std::error::Error for LowerError {}

pub fn lower_function(graph: &FunctionGraph) -> Result<MachineFunction, LowerError> {
    lower_function_with_options(graph, LayoutOptions::default())
}

pub fn lower_function_with_options(
    graph: &FunctionGraph,
    options: LayoutOptions,
) -> Result<MachineFunction, LowerError> {
    let mut function = lower_function_inner(graph, options)?;
    fallthrough::remove_fallthrough_jumps(&mut function.instructions);
    if let Ok(pattern) = std::env::var("KFRC_DUMP_MACHINE")
        && graph.symbol.contains(&pattern)
    {
        eprintln!("encoded machine {} {{", graph.symbol);
        for (position, instruction) in function.instructions.iter().enumerate() {
            eprintln!("  {position:5} {instruction:?}");
        }
        eprintln!("}}");
    }
    Ok(function)
}

fn lower_function_inner(
    graph: &FunctionGraph,
    options: LayoutOptions,
) -> Result<MachineFunction, LowerError> {
    if graph.state != GraphState::Verified {
        return Err(LowerError::NotVerified);
    }
    if graph.level != GraphLevel::Layout {
        return Err(LowerError::Unsupported(
            "semantic graphs must pass through lower_layout before machine lowering".into(),
        ));
    }
    let schedule = Schedule::new(graph)?;
    let mut lowering = Lowering {
        graph,
        schedule: &schedule,
        instructions: Vec::new(),
        emitted: HashSet::new(),
        next_temporary: graph.len() as u32,
        stack_layouts: graph.stack_slots.clone(),
        return_address: None,
        options,
    };
    lowering.run()?;
    let instructions = lowering.instructions;
    if instructions
        .iter()
        .any(|instruction| !matches!(instruction, LowerInstr::Scalar(_)))
    {
        if let Ok(pattern) = std::env::var("KFRC_DUMP_MACHINE")
            && graph.symbol.contains(&pattern)
        {
            eprintln!("memory machine {} {{", graph.symbol);
            for (position, instruction) in instructions.iter().enumerate() {
                eprintln!("  {position:5} {instruction:?}");
            }
            eprintln!("}}");
        }
        let function = compile_memory_function(
            graph,
            instructions,
            lowering.next_temporary,
            &lowering.stack_layouts,
        )?;
        return Ok(function);
    }
    let instructions: Vec<VirtualInstr> = instructions
        .into_iter()
        .map(|instruction| match instruction {
            LowerInstr::Scalar(instruction) => instruction,
            _ => unreachable!("memory instructions use the stack-machine lowering"),
        })
        .collect();
    let intervals = live_intervals(&instructions);
    if let Ok(pattern) = std::env::var("KFRC_DUMP_MACHINE")
        && graph.symbol.contains(&pattern)
    {
        eprintln!("machine {} {{", graph.symbol);
        for (position, instruction) in instructions.iter().enumerate() {
            eprintln!("  {position:5} {instruction:?}");
        }
        for interval in &intervals {
            eprintln!(
                "  interval v{} [{}, {})",
                interval.virtual_reg.0, interval.start.0, interval.end.0
            );
        }
        eprintln!("}}");
    }
    compile_virtual_function(instructions, intervals, graph.symbol.clone())
        .map_err(LowerError::Backend)
}

// -- scheduling -------------------------------------------------------------------------

// -- instruction selection -------------------------------------------------------------------

struct Lowering<'a> {
    graph: &'a FunctionGraph,
    schedule: &'a Schedule,
    instructions: Vec<LowerInstr>,
    emitted: HashSet<NodeId>,
    next_temporary: u32,
    stack_layouts: Vec<SizeAlign>,
    return_address: Option<VirtualReg>,
    options: LayoutOptions,
}

#[derive(Clone, Debug)]
enum LowerInstr {
    Scalar(VirtualInstr),
    StackAddress {
        dst: VirtualReg,
        slot: usize,
    },
    SymbolAddress {
        dst: VirtualReg,
        symbol: String,
        addend: i64,
    },
    ThreadLocalAddress {
        dst: VirtualReg,
        symbol: String,
    },
    LoadVector {
        dst: Xmm,
        address: VirtualValue,
    },
    StoreVector {
        address: VirtualValue,
        src: Xmm,
    },
    Load {
        dst: VirtualReg,
        address: VirtualValue,
        bytes: u8,
    },
    Store {
        address: VirtualValue,
        src: VirtualValue,
        bytes: u8,
    },
    MulOverflow {
        dst: VirtualReg,
        lhs: VirtualValue,
        rhs: VirtualValue,
        signed: bool,
        bits: u8,
    },
    CallPair {
        first: VirtualReg,
        second: VirtualReg,
        target: CallDestination,
        args: Vec<VirtualValue>,
    },
    CallIndirect {
        dst: VirtualReg,
        target: VirtualValue,
        args: Vec<VirtualValue>,
    },
    AbiCall {
        results: Vec<VirtualReg>,
        target: CallDestination,
        args: Vec<(usize, VirtualValue)>,
        vector_args: Vec<(Xmm, VirtualValue)>,
        vector_result: Option<VirtualValue>,
    },
    ReturnPair {
        first: VirtualValue,
        second: VirtualValue,
    },
}

#[derive(Clone, Debug)]
enum CallDestination {
    Direct(String),
    Indirect(VirtualValue),
}

#[derive(Clone, Copy)]
struct ScalarKind {
    bits: u32,
    signed: bool,
}

fn label_of(block: usize) -> u32 {
    block as u32 + 1
}

fn register(node: NodeId) -> VirtualReg {
    VirtualReg(node.0)
}

impl Lowering<'_> {
    fn temporary(&mut self) -> VirtualReg {
        let register = VirtualReg(self.next_temporary);
        self.next_temporary += 1;
        register
    }

    fn push(&mut self, instruction: VirtualInstr) {
        self.instructions.push(LowerInstr::Scalar(instruction));
    }

    fn scalar_kind(&self, ty: TyId) -> Result<ScalarKind, LowerError> {
        match self.graph.oracle.shape(ty) {
            TypeShape::Bool => Ok(ScalarKind {
                bits: 8,
                signed: false,
            }),
            TypeShape::Char => Ok(ScalarKind {
                bits: 32,
                signed: false,
            }),
            TypeShape::Int { width, signed } => {
                let bits = u32::from(width.bits(64));
                if bits > 64 {
                    return Err(LowerError::Unsupported(format!("{bits}-bit integers")));
                }
                Ok(ScalarKind {
                    bits,
                    signed: *signed,
                })
            }
            TypeShape::PlaceAddress { .. }
            | TypeShape::Reference { .. }
            | TypeShape::RawPointer { .. }
                if self.graph.oracle.semantic().thin_pointee(ty).is_some()
                    && self
                        .graph
                        .oracle
                        .size_align(ty)
                        .is_some_and(|layout| layout.size == 8) =>
            {
                Ok(ScalarKind {
                    bits: 64,
                    signed: false,
                })
            }
            TypeShape::Unit | TypeShape::Never | TypeShape::FunctionItem { .. } => Ok(ScalarKind {
                bits: 0,
                signed: false,
            }),
            _ => Err(LowerError::Unsupported(format!(
                "values of type `{}`",
                self.graph.oracle.describe(ty)
            ))),
        }
    }

    fn node_kind(&self, node: NodeId) -> Result<ScalarKind, LowerError> {
        if matches!(self.graph.value_type(node), ValueType::LayoutValue(_)) {
            return Ok(ScalarKind {
                bits: 64,
                signed: false,
            });
        }
        let ty = self
            .graph
            .rust_type(node)
            .ok_or_else(|| LowerError::Malformed(format!("{node} is not a Rust value")))?;
        self.scalar_kind(ty)
    }

    /// Writes `source` into `destination` in canonical form for `kind`.
    fn canonicalize(&mut self, destination: VirtualReg, source: VirtualValue, kind: ScalarKind) {
        if kind.bits == 0 || kind.bits >= 64 {
            self.push(VirtualInstr::Mov {
                dst: destination,
                src: source,
            });
        } else if kind.signed {
            let shifted = self.temporary();
            let amount = i64::from(64 - kind.bits);
            self.push(VirtualInstr::Shl {
                dst: shifted,
                lhs: source,
                rhs: VirtualValue::Imm(amount),
            });
            self.push(VirtualInstr::Shr {
                dst: destination,
                lhs: VirtualValue::Reg(shifted),
                rhs: VirtualValue::Imm(amount),
            });
        } else {
            let mask = ((1u64 << kind.bits) - 1) as i64;
            self.push(VirtualInstr::And {
                dst: destination,
                lhs: source,
                rhs: VirtualValue::Imm(mask),
            });
        }
    }

    /// The operand for a value node, emitting it first if it is a pure node
    /// placed in the current block.
    fn value(&mut self, node: NodeId) -> Result<VirtualValue, LowerError> {
        match self.graph.op(node) {
            Opcode::Const => {
                let bits = self.graph.constant_value(node);
                let kind = self.node_kind(node)?;
                let value = if kind.signed && kind.bits < 64 && kind.bits > 0 {
                    let shift = 64 - kind.bits;
                    (((bits as u64) << shift) as i64) >> shift
                } else {
                    bits as u64 as i64
                };
                return Ok(VirtualValue::Imm(value));
            }
            Opcode::Undefined => return Ok(VirtualValue::Imm(0)),
            _ => {}
        }
        if !self.emitted.contains(&node) {
            self.emit_pure(node)?;
        }
        Ok(VirtualValue::Reg(register(node)))
    }

    fn emit_pure(&mut self, node: NodeId) -> Result<(), LowerError> {
        let op = self.graph.op(node);
        if matches!(op, Opcode::Phi | Opcode::Projection) || op.is_fixed() {
            // Defined by its block (phis, parameters, call results) or by the chain.
            if !self.emitted.contains(&node) {
                return Err(LowerError::Malformed(format!(
                    "{node} used before its definition was scheduled"
                )));
            }
            return Ok(());
        }
        self.emitted.insert(node);
        let inputs = self.graph.inputs(node).to_vec();
        let mut operands = Vec::with_capacity(inputs.len());
        for input in &inputs {
            operands.push(self.value(*input)?);
        }
        let destination = register(node);
        let kind = if matches!(
            op,
            Opcode::PtrOffset | Opcode::StackSlot | Opcode::SymbolAddress
        ) {
            ScalarKind {
                bits: 64,
                signed: false,
            }
        } else {
            self.node_kind(node)?
        };
        let raw = self.temporary();
        match op {
            Opcode::StackSlot => {
                self.instructions.push(LowerInstr::StackAddress {
                    dst: destination,
                    slot: self.graph.payload(node) as usize,
                });
            }
            Opcode::SymbolAddress => {
                let (symbol, addend) =
                    self.graph.symbols[self.graph.payload(node) as usize].clone();
                self.instructions.push(LowerInstr::SymbolAddress {
                    dst: destination,
                    symbol,
                    addend,
                });
            }
            Opcode::PtrOffset => {
                if operands[1] == VirtualValue::Imm(0) {
                    // The typed graph node may change its pointee type. Keep
                    // that node and its control dependencies; only instruction
                    // selection knows that its address bits are unchanged.
                    self.push(VirtualInstr::Mov {
                        dst: destination,
                        src: operands[0],
                    });
                } else {
                    self.push(VirtualInstr::Add {
                        dst: destination,
                        lhs: operands[0],
                        rhs: operands[1],
                    });
                }
            }
            Opcode::MulOverflow => {
                let input_kind = self.node_kind(inputs[0])?;
                self.instructions.push(LowerInstr::MulOverflow {
                    dst: destination,
                    lhs: operands[0],
                    rhs: operands[1],
                    signed: input_kind.signed,
                    bits: input_kind.bits as u8,
                });
            }
            Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor => {
                let (lhs, rhs) = (operands[0], operands[1]);
                self.push(match op {
                    Opcode::Add => VirtualInstr::Add { dst: raw, lhs, rhs },
                    Opcode::Sub => VirtualInstr::Sub { dst: raw, lhs, rhs },
                    Opcode::Mul => VirtualInstr::Mul { dst: raw, lhs, rhs },
                    Opcode::BitAnd => VirtualInstr::And { dst: raw, lhs, rhs },
                    Opcode::BitOr => VirtualInstr::Or { dst: raw, lhs, rhs },
                    _ => VirtualInstr::Xor { dst: raw, lhs, rhs },
                });
                self.canonicalize(destination, VirtualValue::Reg(raw), kind);
            }
            Opcode::ShiftLeft | Opcode::ShiftRightArithmetic | Opcode::ShiftRightLogical => {
                let amount = self.temporary();
                let mask = i64::from(kind.bits.max(1) - 1);
                self.push(VirtualInstr::And {
                    dst: amount,
                    lhs: operands[1],
                    rhs: VirtualValue::Imm(mask),
                });
                let (lhs, rhs) = (operands[0], VirtualValue::Reg(amount));
                self.push(match op {
                    Opcode::ShiftLeft => VirtualInstr::Shl { dst: raw, lhs, rhs },
                    Opcode::ShiftRightArithmetic => VirtualInstr::Shr { dst: raw, lhs, rhs },
                    _ => VirtualInstr::LShr { dst: raw, lhs, rhs },
                });
                self.canonicalize(destination, VirtualValue::Reg(raw), kind);
            }
            Opcode::Negate => {
                self.push(VirtualInstr::Neg {
                    dst: raw,
                    src: operands[0],
                });
                self.canonicalize(destination, VirtualValue::Reg(raw), kind);
            }
            Opcode::BitNot => {
                let operand_ty = self
                    .graph
                    .rust_type(inputs[0])
                    .expect("verified operand type");
                if matches!(self.graph.oracle.shape(operand_ty), TypeShape::Bool) {
                    self.push(VirtualInstr::Xor {
                        dst: destination,
                        lhs: operands[0],
                        rhs: VirtualValue::Imm(1),
                    });
                } else {
                    self.push(VirtualInstr::Not {
                        dst: raw,
                        src: operands[0],
                    });
                    self.canonicalize(destination, VirtualValue::Reg(raw), kind);
                }
            }
            Opcode::IntCast => self.canonicalize(destination, operands[0], kind),
            Opcode::Compare => {
                let condition = match ComparePredicate::from_payload(self.graph.payload(node)) {
                    ComparePredicate::Equal => Condition::Eq,
                    ComparePredicate::NotEqual => Condition::NotEq,
                    ComparePredicate::SignedLess => Condition::SignedLess,
                    ComparePredicate::SignedLessEqual => Condition::SignedLessEq,
                    ComparePredicate::SignedGreater => Condition::SignedGreater,
                    ComparePredicate::SignedGreaterEqual => Condition::SignedGreaterEq,
                    ComparePredicate::UnsignedLess => Condition::UnsignedBelow,
                    ComparePredicate::UnsignedLessEqual => Condition::UnsignedBelowEq,
                    ComparePredicate::UnsignedGreater => Condition::UnsignedAbove,
                    ComparePredicate::UnsignedGreaterEqual => Condition::UnsignedAboveEq,
                };
                self.push(VirtualInstr::Compare {
                    dst: destination,
                    condition,
                    lhs: operands[0],
                    rhs: operands[1],
                });
            }
            other => {
                return Err(LowerError::Unsupported(format!(
                    "{} node {node}",
                    other.name()
                )));
            }
        }
        Ok(())
    }

    fn run(&mut self) -> Result<(), LowerError> {
        let schedule = self.schedule;
        let order = schedule.order.clone();
        for block in order {
            self.push(VirtualInstr::Label(label_of(block)));
            if block == 0 {
                self.emit_parameters()?;
            }
            // Phis of a region head are defined by the incoming jumps.
            let head = schedule.blocks[block].head;
            if let Some(phis) = schedule.phis_of_region.get(&head) {
                for phi in phis {
                    self.emitted.insert(*phi);
                }
            }
            let chain = schedule.blocks[block].chain.clone();
            let (terminator, body) = chain
                .split_last()
                .expect("every block ends in a control node");
            for node in body {
                self.emit_fixed(*node)?;
            }
            // Pure values placed here but used elsewhere are computed before leaving.
            for node in schedule.placed[block].clone() {
                if !self.emitted.contains(&node) {
                    self.emit_pure(node)?;
                }
            }
            self.emit_terminator(*terminator, block)?;
        }
        Ok(())
    }

    fn emit_fixed(&mut self, node: NodeId) -> Result<(), LowerError> {
        let graph = self.graph;
        match graph.op(node) {
            op @ (Opcode::DivSigned
            | Opcode::DivUnsigned
            | Opcode::RemSigned
            | Opcode::RemUnsigned) => {
                let lhs = self.value(graph.input(node, 0))?;
                let rhs = self.value(graph.input(node, 1))?;
                let raw = self.temporary();
                self.push(match op {
                    Opcode::DivSigned => VirtualInstr::Div { dst: raw, lhs, rhs },
                    Opcode::DivUnsigned => VirtualInstr::UDiv { dst: raw, lhs, rhs },
                    Opcode::RemSigned => VirtualInstr::Rem { dst: raw, lhs, rhs },
                    _ => VirtualInstr::URem { dst: raw, lhs, rhs },
                });
                let kind = self.node_kind(node)?;
                self.canonicalize(register(node), VirtualValue::Reg(raw), kind);
                self.emitted.insert(node);
            }
            Opcode::SlotLive | Opcode::SlotDead => {
                self.emitted.insert(node);
            }
            Opcode::Opaque => {
                let value = self.value(graph.input(node, 1))?;
                if let Some(result) = self.schedule.projections.get(&(node, 2)).copied() {
                    self.push(VirtualInstr::Mov {
                        dst: register(result),
                        src: value,
                    });
                    self.emitted.insert(result);
                }
                self.emitted.insert(node);
            }
            Opcode::ThreadLocalAddress => {
                if let Some(result) = self.schedule.projections.get(&(node, 2)).copied() {
                    let (symbol, addend) = &graph.symbols[graph.payload(node) as usize];
                    if *addend != 0 {
                        return Err(LowerError::Malformed("TLS symbol has an addend".into()));
                    }
                    self.instructions.push(LowerInstr::ThreadLocalAddress {
                        dst: register(result),
                        symbol: symbol.clone(),
                    });
                    self.emitted.insert(result);
                }
                self.emitted.insert(node);
            }
            Opcode::Load => {
                let address = self.value(graph.input(node, 1))?;
                let Some(result) = self.schedule.projections.get(&(node, 2)).copied() else {
                    // A nonvolatile load with an unused value has no observable
                    // behavior; keep its control/effect order without code.
                    self.emitted.insert(node);
                    return Ok(());
                };
                let kind = self.node_kind(result)?;
                let raw = self.temporary();
                self.instructions.push(LowerInstr::Load {
                    dst: raw,
                    address,
                    bytes: (kind.bits / 8) as u8,
                });
                self.canonicalize(register(result), VirtualValue::Reg(raw), kind);
                self.emitted.insert(result);
                self.emitted.insert(node);
            }
            Opcode::Store => {
                let address = self.value(graph.input(node, 1))?;
                let value = graph.input(node, 2);
                let src = self.value(value)?;
                let kind = self.node_kind(value)?;
                self.instructions.push(LowerInstr::Store {
                    address,
                    src,
                    bytes: (kind.bits / 8) as u8,
                });
                self.emitted.insert(node);
            }
            Opcode::Call | Opcode::CallIndirect => {
                let (abi, target, first_arg) = if graph.op(node) == Opcode::Call {
                    let callee = &graph.callees[graph.payload(node) as usize];
                    (
                        callee.abi.clone(),
                        CallDestination::Direct(callee.symbol.clone()),
                        1,
                    )
                } else {
                    let target = self.value(graph.input(node, 1))?;
                    (
                        graph.call_abis[graph.payload(node) as usize].clone(),
                        CallDestination::Indirect(target),
                        2,
                    )
                };
                if abi
                    .args
                    .iter()
                    .chain(std::iter::once(&abi.ret))
                    .any(|mode| matches!(mode, PassMode::Cast(_) | PassMode::Vector { .. }))
                {
                    return self.emit_cast_call(node, abi, target, first_arg);
                }
                let mut args = Vec::new();
                let result = self.schedule.projections.get(&(node, 2)).copied();
                if abi.ret == PassMode::Indirect {
                    let ty = match graph.value_type(node) {
                        ValueType::Tuple(elements) => match elements.get(2) {
                            Some(ValueType::Rust(ty) | ValueType::LayoutValue(ty)) => *ty,
                            _ => {
                                return Err(LowerError::Malformed(
                                    "indirect call has no return type".to_owned(),
                                ));
                            }
                        },
                        _ => {
                            return Err(LowerError::Malformed(
                                "indirect call has no tuple result".to_owned(),
                            ));
                        }
                    };
                    let address = result.map(register).unwrap_or_else(|| self.temporary());
                    self.allocate_slot(address, ty)?;
                    args.push(VirtualValue::Reg(address));
                    if let Some(result) = result {
                        self.emitted.insert(result);
                    }
                }
                for (input, mode) in graph.inputs(node)[first_arg..].iter().zip(&abi.args) {
                    let ty = graph.rust_type(*input).ok_or_else(|| {
                        LowerError::Malformed("call argument has no Rust type".to_owned())
                    })?;
                    match mode {
                        PassMode::Ignore => {}
                        PassMode::Direct if self.scalar_kind(ty).is_ok() => {
                            args.push(self.value(*input)?)
                        }
                        PassMode::Direct => {
                            let address = self.value(*input)?;
                            let dst = self.temporary();
                            let bytes = self.direct_bits(ty)?;
                            self.instructions.push(LowerInstr::Load {
                                dst,
                                address,
                                bytes,
                            });
                            args.push(VirtualValue::Reg(dst));
                        }
                        PassMode::Indirect => {
                            // Layout lowering snapshots by-value aggregates before
                            // entering the call; this value is that private address.
                            args.push(self.value(*input)?);
                        }
                        PassMode::Pair => {
                            let address = self.value(*input)?;
                            let fields = *graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                                LowerError::Unsupported(format!(
                                    "missing ScalarPair layout for `{}`",
                                    graph.oracle.describe(ty)
                                ))
                            })?;
                            for field in fields {
                                args.push(self.load_field(address, field.offset, field.ty)?);
                            }
                        }
                        other => {
                            return Err(LowerError::Unsupported(format!(
                                "argument passed as {other:?}"
                            )));
                        }
                    }
                }
                let raw = self.temporary();
                if abi.ret == PassMode::Pair {
                    let second = self.temporary();
                    self.instructions.push(LowerInstr::CallPair {
                        first: raw,
                        second,
                        target,
                        args,
                    });
                    if let Some(result) = result {
                        let ty = graph.rust_type(result).ok_or_else(|| {
                            LowerError::Malformed("call result has no Rust type".to_owned())
                        })?;
                        let fields = *graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                            LowerError::Unsupported(format!(
                                "missing ScalarPair return layout for `{}`",
                                graph.oracle.describe(ty)
                            ))
                        })?;
                        self.allocate_slot(register(result), ty)?;
                        for (field, value) in fields.into_iter().zip([raw, second]) {
                            self.store_field(
                                VirtualValue::Reg(register(result)),
                                field.offset,
                                field.ty,
                                VirtualValue::Reg(value),
                            )?;
                        }
                        self.emitted.insert(result);
                    }
                } else {
                    match target {
                        CallDestination::Direct(target) => self.push(VirtualInstr::Call {
                            dst: raw,
                            target,
                            args,
                        }),
                        CallDestination::Indirect(target) => {
                            self.instructions.push(LowerInstr::CallIndirect {
                                dst: raw,
                                target,
                                args,
                            })
                        }
                    }
                    if let Some(result) = result.filter(|_| abi.ret != PassMode::Indirect) {
                        let ty = graph.rust_type(result).ok_or_else(|| {
                            LowerError::Malformed("call result has no Rust type".to_owned())
                        })?;
                        if let Ok(kind) = self.scalar_kind(ty) {
                            self.canonicalize(register(result), VirtualValue::Reg(raw), kind);
                        } else {
                            self.allocate_slot(register(result), ty)?;
                            let bytes = self.direct_bits(ty)?;
                            self.instructions.push(LowerInstr::Store {
                                address: VirtualValue::Reg(register(result)),
                                src: VirtualValue::Reg(raw),
                                bytes,
                            });
                        }
                        self.emitted.insert(result);
                    }
                }
                self.emitted.insert(node);
            }
            other => {
                return Err(LowerError::Malformed(format!(
                    "{} in the middle of a block",
                    other.name()
                )));
            }
        }
        Ok(())
    }

    fn emit_terminator(&mut self, node: NodeId, block: usize) -> Result<(), LowerError> {
        let graph = self.graph;
        let schedule = self.schedule;
        match graph.op(node) {
            Opcode::Call => {
                // A call whose continuation is the next fixed node: handled as a
                // body node; reaching here means the block ended in a call.
                Err(LowerError::Malformed(format!("block ends in call {node}")))
            }
            Opcode::If => {
                let condition = self.value(graph.input(node, 0))?;
                let successors = &schedule.blocks[block].successors;
                // `Test { lhs, Imm(0) }` is `test`; any other right operand is `cmp`.
                self.push(VirtualInstr::Test {
                    lhs: condition,
                    rhs: VirtualValue::Imm(0),
                });
                self.push(VirtualInstr::Jcc {
                    condition: Condition::NotEq,
                    target: label_of(successors[0]),
                });
                self.push(VirtualInstr::Jmp(label_of(successors[1])));
                Ok(())
            }
            Opcode::Jump => {
                let (region, slot) = schedule.region_of_jump[&node];
                if let Some(phis) = schedule.phis_of_region.get(&region) {
                    let mut moves = Vec::new();
                    for phi in phis {
                        if matches!(graph.value_type(*phi), ValueType::Effect) {
                            continue;
                        }
                        let source = self.value(graph.input(*phi, slot + 1))?;
                        let staged = self.temporary();
                        self.push(VirtualInstr::Mov {
                            dst: staged,
                            src: source,
                        });
                        moves.push((register(*phi), staged));
                    }
                    for (phi, staged) in moves {
                        self.push(VirtualInstr::Mov {
                            dst: phi,
                            src: VirtualValue::Reg(staged),
                        });
                    }
                }
                let target = schedule.block_of_head[&region];
                self.push(VirtualInstr::Jmp(label_of(target)));
                Ok(())
            }
            Opcode::Return => {
                let inputs = graph.inputs(node);
                if let Some(value) = inputs.get(1) {
                    let ty = graph.rust_type(*value).ok_or_else(|| {
                        LowerError::Malformed("return has no Rust type".to_owned())
                    })?;
                    match graph.abi.ret {
                        PassMode::Vector { bytes: 16 } => {
                            let address = self.value(*value)?;
                            self.instructions.push(LowerInstr::LoadVector {
                                dst: Xmm(0),
                                address,
                            });
                            self.push(VirtualInstr::Return(VirtualValue::Imm(0)));
                        }
                        PassMode::Cast(cast) => {
                            let lanes = self.load_cast(*value, cast)?;
                            match lanes.as_slice() {
                                [] => self.push(VirtualInstr::Return(VirtualValue::Imm(0))),
                                [value] => self.push(VirtualInstr::Return(*value)),
                                [first, second] => self.instructions.push(LowerInstr::ReturnPair {
                                    first: *first,
                                    second: *second,
                                }),
                                _ => {
                                    return Err(LowerError::Unsupported(
                                        "Cast return exceeds two GPRs".to_owned(),
                                    ));
                                }
                            }
                        }
                        PassMode::Indirect => {
                            let destination = self.return_address.ok_or_else(|| {
                                LowerError::Malformed("missing hidden return pointer".to_owned())
                            })?;
                            let source = self.value(*value)?;
                            let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                                LowerError::Unsupported("unsized indirect return".to_owned())
                            })?;
                            self.copy_bytes(VirtualValue::Reg(destination), source, layout.size);
                            self.push(VirtualInstr::Return(VirtualValue::Reg(destination)));
                        }
                        PassMode::Pair => {
                            let address = self.value(*value)?;
                            let fields = *graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                                LowerError::Unsupported(
                                    "missing ScalarPair return layout".to_owned(),
                                )
                            })?;
                            let first = self.load_field(address, fields[0].offset, fields[0].ty)?;
                            let second =
                                self.load_field(address, fields[1].offset, fields[1].ty)?;
                            self.instructions
                                .push(LowerInstr::ReturnPair { first, second });
                        }
                        PassMode::Direct if self.scalar_kind(ty).is_ok() => {
                            let value = self.value(*value)?;
                            self.push(VirtualInstr::Return(value));
                        }
                        PassMode::Direct => {
                            let address = self.value(*value)?;
                            let dst = self.temporary();
                            let bytes = self.direct_bits(ty)?;
                            self.instructions.push(LowerInstr::Load {
                                dst,
                                address,
                                bytes,
                            });
                            self.push(VirtualInstr::Return(VirtualValue::Reg(dst)));
                        }
                        PassMode::Ignore => self.push(VirtualInstr::Return(VirtualValue::Imm(0))),
                        other => {
                            return Err(LowerError::Unsupported(format!(
                                "return passed as {other:?}"
                            )));
                        }
                    }
                } else {
                    self.push(VirtualInstr::Return(VirtualValue::Imm(0)));
                }
                Ok(())
            }
            // Undefined behaviour or after a diverging call: any code is
            // correct; return without a value.
            Opcode::Unreachable => {
                self.push(VirtualInstr::Return(VirtualValue::Imm(0)));
                Ok(())
            }
            other => Err(LowerError::Malformed(format!(
                "{} cannot end a block",
                other.name()
            ))),
        }
    }
}

// -- liveness -----------------------------------------------------------------------------------
