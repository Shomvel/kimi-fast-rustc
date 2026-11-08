//! Graph → `VirtualInstr` → `MachineFunction`.

use std::collections::{BTreeSet, HashMap, HashSet};

use kfrc_body::{PassMode, TyId, TypeOracle, TypeShape};
use kfrc_son::{ComparePredicate, FunctionGraph, GraphState, NodeId, Opcode, ValueType};
use kimi_fast_rustc::native::{CompileError, compile_virtual_function};
use kimi_fast_rustc::regalloc::{LiveInterval, VirtualReg as AllocationReg};
use kimi_fast_rustc::x86::{Condition, MachineFunction, VirtualInstr, VirtualReg, VirtualValue};

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
            Self::Backend(error) => write!(formatter, "register allocation or emission failed: {error}"),
        }
    }
}

impl std::error::Error for LowerError {}

pub fn lower_function(graph: &FunctionGraph) -> Result<MachineFunction, LowerError> {
    if graph.state != GraphState::Verified {
        return Err(LowerError::NotVerified);
    }
    let schedule = Schedule::new(graph)?;
    let mut lowering = Lowering {
        graph,
        schedule: &schedule,
        instructions: Vec::new(),
        emitted: HashSet::new(),
        next_temporary: graph.len() as u32,
    };
    lowering.run()?;
    let instructions = lowering.instructions;
    let intervals = live_intervals(&instructions);
    compile_virtual_function(instructions, intervals, graph.symbol.clone()).map_err(LowerError::Backend)
}

// -- scheduling -------------------------------------------------------------------------

struct Block {
    head: NodeId,
    /// Fixed nodes in execution order; the last one ends the block.
    chain: Vec<NodeId>,
    successors: Vec<usize>,
}

struct Schedule {
    blocks: Vec<Block>,
    /// Blocks in emission order (reverse postorder).
    order: Vec<usize>,
    block_of_head: HashMap<NodeId, usize>,
    /// Pure nodes placed in each block.
    placed: Vec<Vec<NodeId>>,
    region_of_jump: HashMap<NodeId, (NodeId, usize)>,
    phis_of_region: HashMap<NodeId, Vec<NodeId>>,
    projections: HashMap<(NodeId, u32), NodeId>,
}

impl Schedule {
    fn new(graph: &FunctionGraph) -> Result<Self, LowerError> {
        let mut control_users: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut region_of_jump = HashMap::new();
        let mut phis_of_region: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut projections = HashMap::new();
        for node in graph.ids() {
            let control = graph.control(node);
            if !control.is_none() {
                control_users.entry(control).or_default().push(node);
            }
            match graph.op(node) {
                Opcode::Region => {
                    for (slot, jump) in graph.inputs(node).iter().enumerate() {
                        region_of_jump.insert(*jump, (node, slot));
                    }
                }
                Opcode::Phi => phis_of_region.entry(graph.input(node, 0)).or_default().push(node),
                Opcode::Projection => {
                    projections.insert((graph.input(node, 0), graph.payload(node)), node);
                }
                _ => {}
            }
        }
        let entry = *projections
            .get(&(graph.start, 0))
            .ok_or_else(|| LowerError::Malformed("start has no control projection".to_owned()))?;

        // Discover blocks from the entry by following control.
        let mut blocks: Vec<Block> = Vec::new();
        let mut block_of_head: HashMap<NodeId, usize> = HashMap::new();
        let mut worklist = vec![entry];
        block_of_head.insert(entry, 0);
        blocks.push(Block { head: entry, chain: Vec::new(), successors: Vec::new() });
        let mut placement: HashMap<NodeId, usize> = HashMap::new();
        while let Some(head) = worklist.pop() {
            let index = block_of_head[&head];
            placement.insert(head, index);
            let mut control = head;
            let mut chain = Vec::new();
            let mut successors_heads = Vec::new();
            loop {
                let users = control_users.get(&control).cloned().unwrap_or_default();
                for user in &users {
                    if matches!(graph.op(*user), Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned) {
                        chain.push(*user);
                    }
                }
                let next = users
                    .iter()
                    .copied()
                    .find(|user| matches!(graph.op(*user), Opcode::Call | Opcode::If | Opcode::Jump | Opcode::Return | Opcode::Unreachable))
                    .ok_or_else(|| LowerError::Malformed(format!("control {control} has no successor")))?;
                chain.push(next);
                match graph.op(next) {
                    Opcode::Call => {
                        control = *projections
                            .get(&(next, 0))
                            .ok_or_else(|| LowerError::Malformed(format!("{next} lacks a control projection")))?;
                        placement.insert(control, index);
                    }
                    Opcode::If => {
                        for branch in [0, 1] {
                            let projection = *projections
                                .get(&(next, branch))
                                .ok_or_else(|| LowerError::Malformed(format!("{next} lacks projection {branch}")))?;
                            successors_heads.push(projection);
                        }
                        break;
                    }
                    Opcode::Jump => {
                        let (region, _) = *region_of_jump
                            .get(&next)
                            .ok_or_else(|| LowerError::Malformed(format!("{next} jumps nowhere")))?;
                        successors_heads.push(region);
                        break;
                    }
                    _ => break,
                }
            }
            let mut successors = Vec::new();
            for successor in successors_heads {
                let successor_index = match block_of_head.get(&successor) {
                    Some(existing) => *existing,
                    None => {
                        let new_index = blocks.len();
                        blocks.push(Block { head: successor, chain: Vec::new(), successors: Vec::new() });
                        block_of_head.insert(successor, new_index);
                        worklist.push(successor);
                        new_index
                    }
                };
                successors.push(successor_index);
            }
            for node in &chain {
                placement.insert(*node, index);
            }
            blocks[index].chain = chain;
            blocks[index].successors = successors;
        }

        let order = reverse_postorder(&blocks);
        let rank: HashMap<usize, usize> = order.iter().enumerate().map(|(rank, block)| (*block, rank)).collect();
        let idom = dominators(&blocks, &order, &rank);
        let depth = dominator_depths(&order, &idom);

        // Place value nodes at their earliest block.
        let mut placed = vec![Vec::new(); blocks.len()];
        let ids: Vec<NodeId> = graph.ids().collect();
        for node in &ids {
            place(graph, *node, &mut placement, &mut placed, &depth)?;
        }
        Ok(Self {
            blocks,
            order,
            block_of_head,
            placed,
            region_of_jump,
            phis_of_region,
            projections,
        })
    }
}

fn place(
    graph: &FunctionGraph,
    node: NodeId,
    placement: &mut HashMap<NodeId, usize>,
    placed: &mut [Vec<NodeId>],
    depth: &HashMap<usize, usize>,
) -> Result<usize, LowerError> {
    if let Some(block) = placement.get(&node) {
        return Ok(*block);
    }
    let block = match graph.op(node) {
        Opcode::Start | Opcode::Const | Opcode::Undefined => 0,
        Opcode::Phi => {
            let region = graph.input(node, 0);
            *placement
                .get(&region)
                .ok_or_else(|| LowerError::Malformed(format!("phi {node} of an unreachable region")))?
        }
        Opcode::Projection => {
            let source = graph.input(node, 0);
            match graph.op(source) {
                Opcode::Start => 0,
                _ => *placement
                    .get(&source)
                    .ok_or_else(|| LowerError::Malformed(format!("projection {node} of unscheduled {source}")))?,
            }
        }
        Opcode::Region | Opcode::Jump | Opcode::If | Opcode::Call | Opcode::Return | Opcode::Unreachable => {
            return Err(LowerError::Malformed(format!("control node {node} is unreachable")));
        }
        _ => {
            let mut deepest = 0;
            for input in graph.inputs(node).to_vec() {
                let input_block = place(graph, input, placement, placed, depth)?;
                if depth.get(&input_block).copied().unwrap_or(0) > depth.get(&deepest).copied().unwrap_or(0) {
                    deepest = input_block;
                }
            }
            placed[deepest].push(node);
            deepest
        }
    };
    placement.insert(node, block);
    Ok(block)
}

fn reverse_postorder(blocks: &[Block]) -> Vec<usize> {
    let mut visited = vec![false; blocks.len()];
    let mut postorder = Vec::new();
    let mut stack = vec![(0usize, 0usize)];
    visited[0] = true;
    while let Some((block, next)) = stack.pop() {
        if next < blocks[block].successors.len() {
            stack.push((block, next + 1));
            let successor = blocks[block].successors[next];
            if !visited[successor] {
                visited[successor] = true;
                stack.push((successor, 0));
            }
        } else {
            postorder.push(block);
        }
    }
    postorder.reverse();
    postorder
}

fn dominators(blocks: &[Block], order: &[usize], rank: &HashMap<usize, usize>) -> Vec<usize> {
    let mut predecessors = vec![Vec::new(); order.len()];
    for (block_rank, block) in order.iter().enumerate() {
        for successor in &blocks[*block].successors {
            predecessors[rank[successor]].push(block_rank);
        }
    }
    let mut idom = vec![usize::MAX; order.len()];
    idom[0] = 0;
    let mut changed = true;
    while changed {
        changed = false;
        for block_rank in 1..order.len() {
            let mut candidate = usize::MAX;
            for &predecessor in &predecessors[block_rank] {
                if idom[predecessor] == usize::MAX {
                    continue;
                }
                candidate = if candidate == usize::MAX {
                    predecessor
                } else {
                    let (mut a, mut b) = (predecessor, candidate);
                    while a != b {
                        while a > b {
                            a = idom[a];
                        }
                        while b > a {
                            b = idom[b];
                        }
                    }
                    a
                };
            }
            if candidate != idom[block_rank] {
                idom[block_rank] = candidate;
                changed = true;
            }
        }
    }
    idom
}

/// Dominator-tree depth per block id.
fn dominator_depths(order: &[usize], idom: &[usize]) -> HashMap<usize, usize> {
    let mut depth_by_rank = vec![0usize; order.len()];
    for rank in 1..order.len() {
        depth_by_rank[rank] = depth_by_rank[idom[rank]] + 1;
    }
    order.iter().enumerate().map(|(rank, block)| (*block, depth_by_rank[rank])).collect()
}

// -- instruction selection -------------------------------------------------------------------

struct Lowering<'a> {
    graph: &'a FunctionGraph,
    schedule: &'a Schedule,
    instructions: Vec<VirtualInstr>,
    emitted: HashSet<NodeId>,
    next_temporary: u32,
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
        self.instructions.push(instruction);
    }

    fn scalar_kind(&self, ty: TyId) -> Result<ScalarKind, LowerError> {
        match self.graph.oracle.shape(ty) {
            TypeShape::Bool => Ok(ScalarKind { bits: 8, signed: false }),
            TypeShape::Char => Ok(ScalarKind { bits: 32, signed: false }),
            TypeShape::Int { width, signed } => {
                let bits = u32::from(width.bits(64));
                if bits > 64 {
                    return Err(LowerError::Unsupported(format!("{bits}-bit integers")));
                }
                Ok(ScalarKind { bits, signed: *signed })
            }
            TypeShape::Unit | TypeShape::Never | TypeShape::FunctionItem { .. } => Ok(ScalarKind { bits: 0, signed: false }),
            _ => Err(LowerError::Unsupported(format!("values of type `{}`", self.graph.oracle.describe(ty)))),
        }
    }

    fn node_kind(&self, node: NodeId) -> Result<ScalarKind, LowerError> {
        let ty = self
            .graph
            .rust_type(node)
            .ok_or_else(|| LowerError::Malformed(format!("{node} is not a Rust value")))?;
        self.scalar_kind(ty)
    }

    /// Writes `source` into `destination` in canonical form for `kind`.
    fn canonicalize(&mut self, destination: VirtualReg, source: VirtualValue, kind: ScalarKind) {
        if kind.bits == 0 || kind.bits >= 64 {
            self.push(VirtualInstr::Mov { dst: destination, src: source });
        } else if kind.signed {
            let shifted = self.temporary();
            let amount = i64::from(64 - kind.bits);
            self.push(VirtualInstr::Shl { dst: shifted, lhs: source, rhs: VirtualValue::Imm(amount) });
            self.push(VirtualInstr::Shr { dst: destination, lhs: VirtualValue::Reg(shifted), rhs: VirtualValue::Imm(amount) });
        } else {
            let mask = ((1u64 << kind.bits) - 1) as i64;
            self.push(VirtualInstr::And { dst: destination, lhs: source, rhs: VirtualValue::Imm(mask) });
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
                return Err(LowerError::Malformed(format!("{node} used before its definition was scheduled")));
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
        let kind = self.node_kind(node)?;
        let raw = self.temporary();
        match op {
            Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::BitAnd | Opcode::BitOr | Opcode::BitXor => {
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
                self.push(VirtualInstr::And { dst: amount, lhs: operands[1], rhs: VirtualValue::Imm(mask) });
                let (lhs, rhs) = (operands[0], VirtualValue::Reg(amount));
                self.push(match op {
                    Opcode::ShiftLeft => VirtualInstr::Shl { dst: raw, lhs, rhs },
                    Opcode::ShiftRightArithmetic => VirtualInstr::Shr { dst: raw, lhs, rhs },
                    _ => VirtualInstr::LShr { dst: raw, lhs, rhs },
                });
                self.canonicalize(destination, VirtualValue::Reg(raw), kind);
            }
            Opcode::Negate => {
                self.push(VirtualInstr::Neg { dst: raw, src: operands[0] });
                self.canonicalize(destination, VirtualValue::Reg(raw), kind);
            }
            Opcode::BitNot => {
                let operand_ty = self.graph.rust_type(inputs[0]).expect("verified operand type");
                if matches!(self.graph.oracle.shape(operand_ty), TypeShape::Bool) {
                    self.push(VirtualInstr::Xor { dst: destination, lhs: operands[0], rhs: VirtualValue::Imm(1) });
                } else {
                    self.push(VirtualInstr::Not { dst: raw, src: operands[0] });
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
                self.push(VirtualInstr::Compare { dst: destination, condition, lhs: operands[0], rhs: operands[1] });
            }
            other => return Err(LowerError::Unsupported(format!("{} node {node}", other.name()))),
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
            let (terminator, body) = chain.split_last().expect("every block ends in a control node");
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

    fn emit_parameters(&mut self) -> Result<(), LowerError> {
        let graph = self.graph;
        let mut passed = 0usize;
        for (index, mode) in graph.abi.args.iter().enumerate() {
            let Some(parameter) = self.schedule.projections.get(&(graph.start, 2 + index as u32)).copied() else {
                if *mode != PassMode::Ignore {
                    passed += 1;
                }
                continue;
            };
            self.emitted.insert(parameter);
            match mode {
                PassMode::Ignore => self.push(VirtualInstr::Mov { dst: register(parameter), src: VirtualValue::Imm(0) }),
                PassMode::Direct => {
                    let raw = self.temporary();
                    self.push(VirtualInstr::LoadArg { dst: raw, index: passed });
                    let kind = self.node_kind(parameter)?;
                    self.canonicalize(register(parameter), VirtualValue::Reg(raw), kind);
                    passed += 1;
                }
                other => return Err(LowerError::Unsupported(format!("parameter passed as {other:?}"))),
            }
        }
        Ok(())
    }

    fn emit_fixed(&mut self, node: NodeId) -> Result<(), LowerError> {
        let graph = self.graph;
        match graph.op(node) {
            op @ (Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned) => {
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
            Opcode::Call => {
                let callee = &graph.callees[graph.payload(node) as usize];
                let mut args = Vec::new();
                for (input, mode) in graph.inputs(node)[1..].iter().zip(&callee.abi.args) {
                    match mode {
                        PassMode::Ignore => {}
                        PassMode::Direct => args.push(self.value(*input)?),
                        other => return Err(LowerError::Unsupported(format!("argument passed as {other:?}"))),
                    }
                }
                let raw = self.temporary();
                self.push(VirtualInstr::Call { dst: raw, target: callee.symbol.clone(), args });
                if let Some(result) = self.schedule.projections.get(&(node, 2)).copied() {
                    let kind = self.node_kind(result)?;
                    self.canonicalize(register(result), VirtualValue::Reg(raw), kind);
                    self.emitted.insert(result);
                }
                self.emitted.insert(node);
            }
            other => return Err(LowerError::Malformed(format!("{} in the middle of a block", other.name()))),
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
                self.push(VirtualInstr::Test { lhs: condition, rhs: VirtualValue::Imm(0) });
                self.push(VirtualInstr::Jcc { condition: Condition::NotEq, target: label_of(successors[0]) });
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
                        self.push(VirtualInstr::Mov { dst: staged, src: source });
                        moves.push((register(*phi), staged));
                    }
                    for (phi, staged) in moves {
                        self.push(VirtualInstr::Mov { dst: phi, src: VirtualValue::Reg(staged) });
                    }
                }
                let target = schedule.block_of_head[&region];
                self.push(VirtualInstr::Jmp(label_of(target)));
                Ok(())
            }
            Opcode::Return => {
                let inputs = graph.inputs(node);
                let value = match inputs.get(1) {
                    Some(value) if graph.abi.ret == PassMode::Direct => self.value(*value)?,
                    _ => VirtualValue::Imm(0),
                };
                self.push(VirtualInstr::Return(value));
                Ok(())
            }
            // Undefined behaviour or after a diverging call: any code is
            // correct; return without a value.
            Opcode::Unreachable => {
                self.push(VirtualInstr::Return(VirtualValue::Imm(0)));
                Ok(())
            }
            other => Err(LowerError::Malformed(format!("{} cannot end a block", other.name()))),
        }
    }
}

// -- liveness -----------------------------------------------------------------------------------

fn defined(instruction: &VirtualInstr) -> Option<VirtualReg> {
    use VirtualInstr::*;
    match instruction {
        LoadArg { dst, .. } | Mov { dst, .. } | Neg { dst, .. } | Not { dst, .. } | Add { dst, .. } | Sub { dst, .. }
        | Mul { dst, .. } | And { dst, .. } | Or { dst, .. } | Xor { dst, .. } | Shl { dst, .. } | Shr { dst, .. }
        | LShr { dst, .. } | Div { dst, .. } | Rem { dst, .. } | UDiv { dst, .. } | URem { dst, .. }
        | Compare { dst, .. } | Load { dst, .. } | Phi { dst } | Call { dst, .. } => Some(*dst),
        Test { .. } | Nop | Store { .. } | Label(_) | Jmp(_) | Jcc { .. } | Return(_) => None,
    }
}

fn used(instruction: &VirtualInstr) -> Vec<VirtualReg> {
    use VirtualInstr::*;
    let mut registers = Vec::new();
    let mut add = |value: &VirtualValue| {
        if let VirtualValue::Reg(register) = value {
            registers.push(*register);
        }
    };
    match instruction {
        Mov { src, .. } | Neg { src, .. } | Not { src, .. } | Return(src) | Store { src, .. } => add(src),
        Add { lhs, rhs, .. } | Sub { lhs, rhs, .. } | Mul { lhs, rhs, .. } | And { lhs, rhs, .. } | Or { lhs, rhs, .. }
        | Xor { lhs, rhs, .. } | Shl { lhs, rhs, .. } | Shr { lhs, rhs, .. } | LShr { lhs, rhs, .. }
        | Div { lhs, rhs, .. } | Rem { lhs, rhs, .. } | UDiv { lhs, rhs, .. } | URem { lhs, rhs, .. }
        | Compare { lhs, rhs, .. } | Test { lhs, rhs } => {
            add(lhs);
            add(rhs);
        }
        Call { args, .. } => args.iter().for_each(&mut add),
        LoadArg { .. } | Load { .. } | Phi { .. } | Nop | Label(_) | Jmp(_) | Jcc { .. } => {}
    }
    registers
}

/// One conservative range per virtual register: the hull of every position
/// where it is defined, used, or live across a block boundary, from a
/// standard backward dataflow over the instruction CFG.
fn live_intervals(instructions: &[VirtualInstr]) -> Vec<LiveInterval> {
    // Split into blocks at labels and after control transfers.
    let mut starts = BTreeSet::new();
    starts.insert(0usize);
    for (index, instruction) in instructions.iter().enumerate() {
        match instruction {
            VirtualInstr::Label(_) => {
                starts.insert(index);
            }
            VirtualInstr::Jmp(_) | VirtualInstr::Jcc { .. } | VirtualInstr::Return(_) => {
                starts.insert(index + 1);
            }
            _ => {}
        }
    }
    starts.retain(|start| *start < instructions.len());
    let starts: Vec<usize> = starts.into_iter().collect();
    let block_count = starts.len();
    let end_of = |block: usize| starts.get(block + 1).copied().unwrap_or(instructions.len());
    let block_at: HashMap<usize, usize> = starts.iter().enumerate().map(|(block, start)| (*start, block)).collect();
    let label_block: HashMap<u32, usize> = instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| match instruction {
            VirtualInstr::Label(label) => Some((*label, block_at[&index])),
            _ => None,
        })
        .collect();
    let mut successors = vec![Vec::new(); block_count];
    for block in 0..block_count {
        let end = end_of(block);
        let mut falls_through = true;
        for instruction in &instructions[starts[block]..end] {
            match instruction {
                VirtualInstr::Jmp(label) => {
                    successors[block].push(label_block[label]);
                    falls_through = false;
                }
                VirtualInstr::Jcc { target, .. } => successors[block].push(label_block[target]),
                VirtualInstr::Return(_) => falls_through = false,
                _ => {}
            }
        }
        if falls_through && block + 1 < block_count {
            successors[block].push(block + 1);
        }
    }
    // Upward-exposed uses and definitions per block.
    let mut uses = vec![HashSet::new(); block_count];
    let mut defs = vec![HashSet::new(); block_count];
    for block in 0..block_count {
        for instruction in &instructions[starts[block]..end_of(block)] {
            for register in used(instruction) {
                if !defs[block].contains(&register) {
                    uses[block].insert(register);
                }
            }
            if let Some(register) = defined(instruction) {
                defs[block].insert(register);
            }
        }
    }
    let mut live_in: Vec<HashSet<VirtualReg>> = vec![HashSet::new(); block_count];
    let mut live_out: Vec<HashSet<VirtualReg>> = vec![HashSet::new(); block_count];
    let mut changed = true;
    while changed {
        changed = false;
        for block in (0..block_count).rev() {
            let mut out = HashSet::new();
            for successor in &successors[block] {
                out.extend(live_in[*successor].iter().copied());
            }
            let mut incoming = uses[block].clone();
            for register in &out {
                if !defs[block].contains(register) {
                    incoming.insert(*register);
                }
            }
            if incoming != live_in[block] || out != live_out[block] {
                live_in[block] = incoming;
                live_out[block] = out;
                changed = true;
            }
        }
    }
    let mut ranges: HashMap<u32, (u32, u32)> = HashMap::new();
    let mut extend = |register: VirtualReg, position: usize| {
        let position = position as u32;
        ranges
            .entry(register.0)
            .and_modify(|(start, end)| {
                *start = (*start).min(position);
                *end = (*end).max(position + 1);
            })
            .or_insert((position, position + 1));
    };
    for block in 0..block_count {
        let (start, end) = (starts[block], end_of(block));
        for register in &live_in[block] {
            extend(*register, start);
        }
        for register in &live_out[block] {
            extend(*register, end - 1);
        }
        for (index, instruction) in instructions[start..end].iter().enumerate() {
            for register in used(instruction) {
                extend(register, start + index);
            }
            if let Some(register) = defined(instruction) {
                extend(register, start + index);
            }
        }
    }
    let mut intervals: Vec<LiveInterval> = ranges
        .into_iter()
        .map(|(register, (start, end))| LiveInterval::new(AllocationReg(register), start, end))
        .collect();
    intervals.sort_by_key(|interval| (interval.start, interval.virtual_reg.0));
    intervals
}
