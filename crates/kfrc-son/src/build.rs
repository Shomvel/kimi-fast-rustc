//! Typed Body → R-level graph: the single production entry (T09).
//!
//! SSA is constructed once, here, from the Body's CFG with the algorithm of
//! Braun et al., "Simple and Efficient Construction of Static Single
//! Assignment Form" (CC 2013): blocks are visited in reverse postorder, a
//! block is sealed when all its incoming edges exist, and values are merged
//! at `Region`s by `Phi`s indexed by predecessor slot. Every variable — each
//! local and the effect token — goes through the same read/write path, so no
//! control construct merges variables on its own (the root cause of the T01
//! miscompile class).
//!
//! Unwind edges are not followed: the PoC only compiles crates with
//! `-Cpanic=abort` (`docs/plan/poc/scope.md`), where they are never taken.

use std::collections::HashMap;

use kfrc_body::{
    BinaryOp, BlockId, Body, Callee, CastKind, ConstantKind, IntegerWidth, Operand, PassMode,
    Place, Rvalue, StatementKind, Terminator, TyId, TypeOracle, TypeShape, UnaryOp,
    verify::{BodyError, verify_body},
};

use crate::graph::{
    CallTarget, ComparePredicate, FunctionGraph, GraphState, NodeId, Opcode, ValueType,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    InvalidBody(BodyError),
    /// Valid input outside the implemented subset.
    Unsupported { block: Option<BlockId>, what: String },
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBody(error) => error.fmt(formatter),
            Self::Unsupported { block: Some(block), what } => {
                write!(formatter, "unsupported in bb{}: {what}", block.0)
            }
            Self::Unsupported { block: None, what } => write!(formatter, "unsupported: {what}"),
        }
    }
}

impl std::error::Error for BuildError {}

pub fn build_graph(body: &Body) -> Result<FunctionGraph, BuildError> {
    verify_body(body).map_err(BuildError::InvalidBody)?;
    let mut builder = Builder::new(body)?;
    builder.run()?;
    let mut graph = builder.graph;
    graph.compact();
    graph.state = GraphState::Sealed;
    Ok(graph)
}

/// Where a block's incoming edge comes from: a Body block or the function entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EdgeSource {
    Entry,
    Block(BlockId),
}

type Variable = u32;

struct Builder<'a> {
    body: &'a Body,
    graph: FunctionGraph,
    /// Reachable blocks in reverse postorder.
    order: Vec<BlockId>,
    /// Incoming edges per block, in slot order (`Entry` first for bb0).
    incoming: Vec<Vec<EdgeSource>>,
    region: Vec<NodeId>,
    entry_control: Vec<NodeId>,
    filled: Vec<usize>,
    sealed: Vec<bool>,
    /// Definitions per block; index `blocks.len()` is the entry pseudo-block.
    definitions: Vec<HashMap<Variable, NodeId>>,
    incomplete: Vec<Vec<(Variable, NodeId)>>,
    projections: HashMap<(NodeId, u32), NodeId>,
    undefined: HashMap<Variable, NodeId>,
    effect: Variable,
    current_block: Option<BlockId>,
}

impl<'a> Builder<'a> {
    fn new(body: &'a Body) -> Result<Self, BuildError> {
        let block_count = body.blocks.len();
        let order = reverse_postorder(body);
        let mut incoming = vec![Vec::new(); block_count];
        incoming[0].push(EdgeSource::Entry);
        for block in &order {
            for successor in body.block(*block).terminator.successors() {
                incoming[successor.index()].push(EdgeSource::Block(*block));
            }
        }
        let graph = FunctionGraph::new(body.symbol.clone(), body.abi.clone(), body.types.clone());
        Ok(Self {
            body,
            graph,
            order,
            incoming,
            region: vec![NodeId::NONE; block_count],
            entry_control: vec![NodeId::NONE; block_count],
            filled: vec![0; block_count],
            sealed: vec![false; block_count + 1],
            definitions: vec![HashMap::new(); block_count + 1],
            incomplete: vec![Vec::new(); block_count],
            projections: HashMap::new(),
            undefined: HashMap::new(),
            effect: body.locals.len() as Variable,
            current_block: None,
        })
    }

    fn unsupported<T>(&self, what: impl Into<String>) -> Result<T, BuildError> {
        Err(BuildError::Unsupported {
            block: self.current_block,
            what: what.into(),
        })
    }

    fn entry_pseudo_block(&self) -> usize {
        self.body.blocks.len()
    }

    fn variable_type(&self, variable: Variable) -> ValueType {
        if variable == self.effect {
            ValueType::Effect
        } else {
            ValueType::Rust(self.body.locals[variable as usize].ty)
        }
    }

    fn projection(&mut self, source: NodeId, index: u32, ty: ValueType) -> NodeId {
        if let Some(existing) = self.projections.get(&(source, index)) {
            return *existing;
        }
        let node = self.graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty);
        self.projections.insert((source, index), node);
        node
    }

    fn run(&mut self) -> Result<(), BuildError> {
        self.check_types()?;
        // Start: (Control, Effect, arguments…).
        let mut start_types = vec![ValueType::Control, ValueType::Effect];
        for index in 1..=self.body.arg_count {
            start_types.push(ValueType::Rust(self.body.locals[index].ty));
        }
        let start = self.graph.add_node(Opcode::Start, NodeId::NONE, &[], 0, ValueType::Tuple(start_types));
        self.graph.start = start;
        let start_control = self.projection(start, 0, ValueType::Control);
        let start_effect = self.projection(start, 1, ValueType::Effect);
        let entry = self.entry_pseudo_block();
        self.definitions[entry].insert(self.effect, start_effect);
        for index in 1..=self.body.arg_count {
            let ty = ValueType::Rust(self.body.locals[index].ty);
            let parameter = self.projection(start, 1 + index as u32, ty);
            self.definitions[entry].insert(index as Variable, parameter);
        }
        self.sealed[entry] = true;

        for block in 0..self.body.blocks.len() {
            if self.incoming[block].len() > 1 {
                let count = self.incoming[block].len();
                self.region[block] = self.graph.add_growable_node(Opcode::Region, NodeId::NONE, count, 0, ValueType::Control);
                self.entry_control[block] = self.region[block];
            }
        }
        self.add_edge(EdgeSource::Entry, BlockId(0), start_control);

        for position in 0..self.order.len() {
            let block = self.order[position];
            self.current_block = Some(block);
            self.build_block(block)?;
        }
        Ok(())
    }

    /// Rejects unsupported local types up front with a precise message.
    fn check_types(&self) -> Result<(), BuildError> {
        for (index, local) in self.body.locals.iter().enumerate() {
            match self.body.types.shape(local.ty) {
                TypeShape::Bool | TypeShape::Char | TypeShape::Unit | TypeShape::Never => {}
                TypeShape::Int { width, .. } if *width != IntegerWidth::W128 => {}
                TypeShape::FunctionItem { .. } => {}
                _ => {
                    return Err(BuildError::Unsupported {
                        block: None,
                        what: format!(
                            "local _{index} has type `{}` (P0 supports integers up to 64 bits, bool, char and unit)",
                            self.body.types.describe(local.ty)
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    fn add_edge(&mut self, source: EdgeSource, target: BlockId, control: NodeId) {
        let index = target.index();
        if self.region[index].is_none() {
            self.entry_control[index] = control;
            self.filled[index] = 1;
            self.seal(target);
            return;
        }
        let slot = self.filled[index];
        debug_assert_eq!(self.incoming[index][slot], source, "edges arrive in incoming order");
        let jump = self.graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
        self.graph.set_input(self.region[index], slot, jump);
        self.filled[index] += 1;
        if self.filled[index] == self.incoming[index].len() {
            self.seal(target);
        }
    }

    // -- SSA construction -------------------------------------------------------------

    fn write_variable(&mut self, variable: Variable, block: usize, value: NodeId) {
        self.definitions[block].insert(variable, value);
    }

    fn read_variable(&mut self, variable: Variable, block: usize) -> NodeId {
        if let Some(value) = self.definitions[block].get(&variable) {
            return self.graph.resolve(*value);
        }
        self.read_variable_recursive(variable, block)
    }

    fn read_source(&mut self, variable: Variable, source: EdgeSource) -> NodeId {
        match source {
            EdgeSource::Entry => self.read_variable(variable, self.entry_pseudo_block()),
            EdgeSource::Block(block) => self.read_variable(variable, block.index()),
        }
    }

    fn undefined(&mut self, variable: Variable) -> NodeId {
        if let Some(node) = self.undefined.get(&variable) {
            return *node;
        }
        let ty = self.variable_type(variable);
        let node = self.graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ty);
        self.undefined.insert(variable, node);
        node
    }

    fn read_variable_recursive(&mut self, variable: Variable, block: usize) -> NodeId {
        let value = if block == self.entry_pseudo_block() {
            self.undefined(variable)
        } else if !self.sealed[block] {
            let phi = self.new_phi(variable, block);
            self.incomplete[block].push((variable, phi));
            phi
        } else if self.incoming[block].len() == 1 {
            let source = self.incoming[block][0];
            self.read_source(variable, source)
        } else {
            let phi = self.new_phi(variable, block);
            self.write_variable(variable, block, phi);
            self.add_phi_operands(variable, block, phi)
        };
        self.write_variable(variable, block, value);
        value
    }

    fn new_phi(&mut self, variable: Variable, block: usize) -> NodeId {
        let ty = self.variable_type(variable);
        let count = self.incoming[block].len();
        let phi = self.graph.add_growable_node(Opcode::Phi, NodeId::NONE, count + 1, 0, ty);
        self.graph.set_input(phi, 0, self.region[block]);
        phi
    }

    fn add_phi_operands(&mut self, variable: Variable, block: usize, phi: NodeId) -> NodeId {
        for slot in 0..self.incoming[block].len() {
            let source = self.incoming[block][slot];
            let value = self.read_source(variable, source);
            self.graph.set_input(phi, slot + 1, value);
        }
        self.try_remove_trivial_phi(phi)
    }

    fn try_remove_trivial_phi(&mut self, phi: NodeId) -> NodeId {
        let mut same = NodeId::NONE;
        let operands: Vec<NodeId> = self.graph.inputs(phi)[1..].to_vec();
        for operand in operands {
            let operand = self.graph.resolve(operand);
            if operand == same || operand == phi {
                continue;
            }
            if !same.is_none() {
                return phi;
            }
            same = operand;
        }
        if same.is_none() {
            // Unreachable or only self-referencing: the value is undefined.
            let ty = self.graph.value_type(phi).clone();
            same = self.graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ty);
        }
        self.graph.replace(phi, same);
        same
    }

    fn seal(&mut self, block: BlockId) {
        let index = block.index();
        let pending = std::mem::take(&mut self.incomplete[index]);
        for (variable, phi) in pending {
            self.add_phi_operands(variable, index, phi);
        }
        self.sealed[index] = true;
    }

    // -- blocks ---------------------------------------------------------------------------

    fn build_block(&mut self, block: BlockId) -> Result<(), BuildError> {
        let index = block.index();
        let mut control = self.entry_control[index];
        debug_assert!(!control.is_none(), "reverse postorder visits a block after an incoming edge");
        for statement in self.body.block_statements(block) {
            match statement {
                StatementKind::Assign(place, rvalue) => {
                    let local = self.local_place(*place)?;
                    let ty = self.body.locals[local as usize].ty;
                    let value = self.rvalue(rvalue, ty, index, &mut control)?;
                    self.write_variable(local, index, value);
                }
                StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::Nop => {}
                StatementKind::SetDiscriminant(..) => return self.unsupported("SetDiscriminant"),
            }
        }
        let terminator = self.body.block(block).terminator.clone();
        self.terminator(&terminator, block, control)
    }

    fn local_place(&self, place: Place) -> Result<Variable, BuildError> {
        if !place.is_local() {
            return self.unsupported(format!("place with projections `{}`", self.body.place_text(place)));
        }
        Ok(place.local.0)
    }

    fn operand(&mut self, operand: &Operand, block: usize) -> Result<NodeId, BuildError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let local = self.local_place(*place)?;
                Ok(self.read_variable(local, block))
            }
            Operand::Constant(id) => {
                let constant = self.body.constant(*id);
                match &constant.kind {
                    ConstantKind::Scalar(bits) => Ok(self.graph.add_constant(*bits, constant.ty)),
                    ConstantKind::ZeroSized | ConstantKind::Function(_) => Ok(self.graph.add_constant(0, constant.ty)),
                }
            }
        }
    }

    fn operand_type(&self, operand: &Operand) -> TyId {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.body.locals[place.local.index()].ty,
            Operand::Constant(id) => self.body.constant(*id).ty,
        }
    }

    fn is_signed(&self, ty: TyId) -> bool {
        matches!(self.body.types.shape(ty), TypeShape::Int { signed: true, .. })
    }

    fn rvalue(&mut self, rvalue: &Rvalue, ty: TyId, block: usize, control: &mut NodeId) -> Result<NodeId, BuildError> {
        match rvalue {
            Rvalue::Use(operand) => self.operand(operand, block),
            Rvalue::BinaryOp(op, lhs, rhs) => {
                let signed = self.is_signed(self.operand_type(lhs));
                let left = self.operand(lhs, block)?;
                let right = self.operand(rhs, block)?;
                let opcode = match op {
                    BinaryOp::Add | BinaryOp::AddUnchecked => Opcode::Add,
                    BinaryOp::Sub | BinaryOp::SubUnchecked => Opcode::Sub,
                    BinaryOp::Mul | BinaryOp::MulUnchecked => Opcode::Mul,
                    BinaryOp::Div if signed => Opcode::DivSigned,
                    BinaryOp::Div => Opcode::DivUnsigned,
                    BinaryOp::Rem if signed => Opcode::RemSigned,
                    BinaryOp::Rem => Opcode::RemUnsigned,
                    BinaryOp::BitAnd => Opcode::BitAnd,
                    BinaryOp::BitOr => Opcode::BitOr,
                    BinaryOp::BitXor => Opcode::BitXor,
                    BinaryOp::Shl | BinaryOp::ShlUnchecked => Opcode::ShiftLeft,
                    BinaryOp::Shr | BinaryOp::ShrUnchecked if signed => Opcode::ShiftRightArithmetic,
                    BinaryOp::Shr | BinaryOp::ShrUnchecked => Opcode::ShiftRightLogical,
                    BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                        let predicate = compare_predicate(*op, signed);
                        return Ok(self.graph.add_node(
                            Opcode::Compare,
                            NodeId::NONE,
                            &[left, right],
                            predicate as u32,
                            ValueType::Rust(ty),
                        ));
                    }
                    BinaryOp::AddWithOverflow | BinaryOp::SubWithOverflow | BinaryOp::MulWithOverflow => {
                        return self.unsupported(format!("{op:?} (overflow checks produce a (value, bool) pair; build with -Coverflow-checks=off for P0)"));
                    }
                    BinaryOp::Cmp | BinaryOp::Offset => return self.unsupported(format!("binary operator {op:?}")),
                };
                // Division may trap: fix it on the control skeleton.
                let fixed_control = if opcode.is_fixed() { *control } else { NodeId::NONE };
                Ok(self.graph.add_node(opcode, fixed_control, &[left, right], 0, ValueType::Rust(ty)))
            }
            Rvalue::UnaryOp(op, operand) => {
                let value = self.operand(operand, block)?;
                let opcode = match op {
                    UnaryOp::Not => Opcode::BitNot,
                    UnaryOp::Neg => Opcode::Negate,
                    UnaryOp::PtrMetadata => return self.unsupported("PtrMetadata"),
                };
                Ok(self.graph.add_node(opcode, NodeId::NONE, &[value], 0, ValueType::Rust(ty)))
            }
            Rvalue::Cast(CastKind::IntToInt, operand, target) => {
                let value = self.operand(operand, block)?;
                Ok(self.graph.add_node(Opcode::IntCast, NodeId::NONE, &[value], 0, ValueType::Rust(*target)))
            }
            Rvalue::Cast(kind, _, target) => self.unsupported(format!(
                "cast {kind:?} to `{}`",
                self.body.types.describe(*target)
            )),
            Rvalue::Aggregate(kind, operands) if operands.is_empty() && matches!(self.body.types.shape(ty), TypeShape::Unit) => {
                let _ = kind;
                Ok(self.graph.add_constant(0, ty))
            }
            Rvalue::Aggregate(kind, _) => self.unsupported(format!("aggregate {kind:?} (aggregates arrive in P1)")),
            Rvalue::Ref(..) | Rvalue::RawPointer(..) => self.unsupported("borrowing a local (stack slots arrive in P1)"),
            Rvalue::Discriminant(_) => self.unsupported("discriminant read (enums arrive in P1)"),
            Rvalue::Len(_) => self.unsupported("Len"),
            Rvalue::Unsupported(what) => self.unsupported(what.clone()),
        }
    }

    fn terminator(&mut self, terminator: &Terminator, block: BlockId, control: NodeId) -> Result<(), BuildError> {
        let index = block.index();
        let source = EdgeSource::Block(block);
        match terminator {
            Terminator::Goto(target) => {
                self.add_edge(source, *target, control);
                Ok(())
            }
            Terminator::SwitchInt { discriminant, targets } => {
                let discriminant_ty = self.operand_type(discriminant);
                let value = self.operand(discriminant, index)?;
                let bool_ty = self.bool_type();
                let mut current = control;
                for (case, target) in &targets.cases {
                    let condition = if matches!(self.body.types.shape(discriminant_ty), TypeShape::Bool) && *case == 1 {
                        value
                    } else if matches!(self.body.types.shape(discriminant_ty), TypeShape::Bool) && *case == 0 {
                        self.graph.add_node(Opcode::BitNot, NodeId::NONE, &[value], 0, ValueType::Rust(discriminant_ty))
                    } else {
                        let constant = self.graph.add_constant(*case, discriminant_ty);
                        self.graph.add_node(
                            Opcode::Compare,
                            NodeId::NONE,
                            &[value, constant],
                            ComparePredicate::Equal as u32,
                            ValueType::Rust(bool_ty),
                        )
                    };
                    let branch = self.graph.add_node(
                        Opcode::If,
                        current,
                        &[condition],
                        0,
                        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
                    );
                    let taken = self.projection(branch, 0, ValueType::Control);
                    current = self.projection(branch, 1, ValueType::Control);
                    self.add_edge(source, *target, taken);
                }
                self.add_edge(source, targets.otherwise, current);
                Ok(())
            }
            Terminator::Return => {
                let effect = self.read_variable(self.effect, index);
                let mut inputs = vec![effect];
                if self.body.abi.ret != PassMode::Ignore {
                    inputs.push(self.read_variable(0, index));
                }
                self.graph.add_node(Opcode::Return, control, &inputs, 0, ValueType::Control);
                Ok(())
            }
            Terminator::Unreachable => {
                self.graph.add_node(Opcode::Unreachable, control, &[], 0, ValueType::Control);
                Ok(())
            }
            Terminator::Call { callee, args, destination, target, .. } => {
                let Operand::Constant(constant) = callee else {
                    return self.unsupported("indirect call through a function pointer");
                };
                let (symbol, abi) = match &self.body.constant(*constant).kind {
                    ConstantKind::Function(Callee::Function { symbol, abi }) => (symbol.clone(), abi.clone()),
                    ConstantKind::Function(Callee::Intrinsic { name }) => {
                        return self.unsupported(format!("call to intrinsic `{name}`"));
                    }
                    _ => return self.unsupported("call through a non-function constant"),
                };
                if abi.args.iter().chain(std::iter::once(&abi.ret)).any(|mode| !matches!(mode, PassMode::Direct | PassMode::Ignore)) {
                    return self.unsupported(format!("call to `{symbol}` with ABI {:?} (P0 passes scalars only)", abi));
                }
                let destination_local = self.local_place(*destination)?;
                let destination_ty = self.body.locals[destination_local as usize].ty;
                let mut inputs = vec![self.read_variable(self.effect, index)];
                for arg in args {
                    inputs.push(self.operand(arg, index)?);
                }
                let ignore_return = abi.ret == PassMode::Ignore;
                let callee_index = self.graph.add_callee(CallTarget { symbol, abi });
                let call = self.graph.add_node(
                    Opcode::Call,
                    control,
                    &inputs,
                    callee_index,
                    ValueType::Tuple(vec![ValueType::Control, ValueType::Effect, ValueType::Rust(destination_ty)]),
                );
                let after = self.projection(call, 0, ValueType::Control);
                let effect = self.projection(call, 1, ValueType::Effect);
                self.write_variable(self.effect, index, effect);
                let result = if ignore_return {
                    self.graph.add_constant(0, destination_ty)
                } else {
                    self.projection(call, 2, ValueType::Rust(destination_ty))
                };
                self.write_variable(destination_local, index, result);
                match target {
                    Some(target) => self.add_edge(source, *target, after),
                    None => {
                        self.graph.add_node(Opcode::Unreachable, after, &[], 0, ValueType::Control);
                    }
                }
                Ok(())
            }
            Terminator::Assert { message, .. } => self.unsupported(format!(
                "Assert ({message:?}): the panic path needs read-only data and the panic runtime (next P0 step)"
            )),
            Terminator::Drop { .. } => self.unsupported("Drop (drop elaboration arrives in P1)"),
            Terminator::UnwindResume | Terminator::UnwindTerminate => {
                self.unsupported("unwind terminator on a normal path")
            }
        }
    }

    fn bool_type(&mut self) -> TyId {
        self.graph.oracle.intern(TypeShape::Bool, Some(kfrc_body::SizeAlign { size: 1, align: 1 }))
    }
}

fn compare_predicate(op: BinaryOp, signed: bool) -> ComparePredicate {
    use ComparePredicate::*;
    match (op, signed) {
        (BinaryOp::Eq, _) => Equal,
        (BinaryOp::Ne, _) => NotEqual,
        (BinaryOp::Lt, true) => SignedLess,
        (BinaryOp::Le, true) => SignedLessEqual,
        (BinaryOp::Gt, true) => SignedGreater,
        (BinaryOp::Ge, true) => SignedGreaterEqual,
        (BinaryOp::Lt, false) => UnsignedLess,
        (BinaryOp::Le, false) => UnsignedLessEqual,
        (BinaryOp::Gt, false) => UnsignedGreater,
        (BinaryOp::Ge, false) => UnsignedGreaterEqual,
        _ => unreachable!("not a comparison"),
    }
}

/// Blocks reachable from bb0 over normal edges, in reverse postorder.
fn reverse_postorder(body: &Body) -> Vec<BlockId> {
    let count = body.blocks.len();
    let mut visited = vec![false; count];
    let mut postorder = Vec::with_capacity(count);
    let mut stack: Vec<(BlockId, usize)> = vec![(BlockId(0), 0)];
    visited[0] = true;
    while let Some((block, next)) = stack.pop() {
        let successors = body.block(block).terminator.successors();
        if next < successors.len() {
            stack.push((block, next + 1));
            let successor = successors[next];
            if !visited[successor.index()] {
                visited[successor.index()] = true;
                stack.push((successor, 0));
            }
        } else {
            postorder.push(block);
        }
    }
    postorder.reverse();
    postorder
}
