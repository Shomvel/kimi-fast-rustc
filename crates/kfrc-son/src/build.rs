//! Concrete Typed Body → Rust-value graph: the only production build entry.
//!
//! Values retain their logical fields and variants; physical layout is chosen
//! by lower_layout. Definition-level polymorphic R remains a separate milestone.
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

mod calls;
mod control;
mod memory;
mod semantic;
mod values;

use kfrc_body::{
    AggregateKind, BinaryOp, BlockId, Body, Callee, CastKind, ConstantKind, FunctionAbi,
    IntegerWidth, LocalId, Operand, PassMode, Place, ProjectionElem, Rvalue, SizeAlign,
    StatementKind, Terminator, TyId, TypeOracle, TypeShape, UnaryOp,
    verify::{BodyError, verify_body},
};

use crate::graph::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, GraphState, NodeId, Opcode, ValueType,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    InvalidBody(BodyError),
    /// Valid input outside the implemented subset.
    Unsupported {
        block: Option<BlockId>,
        what: String,
    },
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBody(error) => error.fmt(formatter),
            Self::Unsupported {
                block: Some(block),
                what,
            } => {
                write!(formatter, "unsupported in bb{}: {what}", block.0)
            }
            Self::Unsupported { block: None, what } => write!(formatter, "unsupported: {what}"),
        }
    }
}

impl std::error::Error for BuildError {}

/// Build a monomorphic Rust-value graph, without selecting aggregate layouts.
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
    memory_locals: Vec<bool>,
    local_slots: Vec<NodeId>,
    /// Field extractions and their source execution anchors, before proving
    /// any branch-local variant fact against the completed SSA control graph.
    variant_witness_candidates: Vec<(NodeId, NodeId)>,
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
        let mut graph =
            FunctionGraph::new(body.symbol.clone(), body.abi.clone(), body.types.clone());
        graph.data_objects = body.data_objects.clone();
        graph.level = GraphLevel::Specialized;
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
            memory_locals: memory_locals(body),
            local_slots: vec![NodeId::NONE; body.locals.len()],
            variant_witness_candidates: Vec::new(),
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
        let node = self
            .graph
            .add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty);
        self.projections.insert((source, index), node);
        node
    }

    fn run(&mut self) -> Result<(), BuildError> {
        if self.body.statements.iter().any(|statement| {
            matches!(
                statement,
                StatementKind::Assign(_, Rvalue::ThreadLocalRef { .. })
            )
        }) {
            return self.unsupported("TLS is outside the semantic path");
        }
        self.check_semantic_types()?;
        self.check_abi(&self.body.abi)?;
        // Start: (Control, Effect, arguments…).
        let mut start_types = vec![ValueType::Control, ValueType::Effect];
        for index in 1..=self.body.arg_count {
            start_types.push(ValueType::Rust(self.body.locals[index].ty));
        }
        let start = self.graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(start_types),
        );
        self.graph.start = start;
        let mut start_control = self.projection(start, 0, ValueType::Control);
        let start_effect = self.projection(start, 1, ValueType::Effect);
        let entry = self.entry_pseudo_block();
        self.definitions[entry].insert(self.effect, start_effect);
        for index in 1..=self.body.arg_count {
            let ty = ValueType::Rust(self.body.locals[index].ty);
            let parameter = self.projection(start, 1 + index as u32, ty);
            self.definitions[entry].insert(index as Variable, parameter);
        }
        self.sealed[entry] = true;
        for index in 0..self.body.locals.len() {
            if !self.memory_locals[index] {
                continue;
            }
            let ty = self.body.locals[index].ty;
            let pointer_ty = self.pointer_type(ty);
            let address = self.graph.add_node(
                Opcode::LocalSlot,
                NodeId::NONE,
                &[],
                index as u32,
                ValueType::Rust(pointer_ty),
            );
            self.local_slots[index] = address;
            if index > 0 && index <= self.body.arg_count {
                let parameter = self.read_variable(index as Variable, entry);
                self.store(address, parameter, entry, &mut start_control);
            }
        }

        for block in 0..self.body.blocks.len() {
            if self.incoming[block].len() > 1 {
                let count = self.incoming[block].len();
                self.region[block] = self.graph.add_growable_node(
                    Opcode::Region,
                    NodeId::NONE,
                    count,
                    0,
                    ValueType::Control,
                );
                self.entry_control[block] = self.region[block];
            }
        }
        self.add_edge(EdgeSource::Entry, BlockId(0), start_control);

        for position in 0..self.order.len() {
            let block = self.order[position];
            self.current_block = Some(block);
            self.build_block(block)?;
        }
        self.witness_variant_fields();
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
        debug_assert_eq!(
            self.incoming[index][slot], source,
            "edges arrive in incoming order"
        );
        let jump = self
            .graph
            .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
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
        let node = self
            .graph
            .add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ty);
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
        let phi = self
            .graph
            .add_growable_node(Opcode::Phi, NodeId::NONE, count + 1, 0, ty);
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
            same = self
                .graph
                .add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ty);
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
        debug_assert!(
            !control.is_none(),
            "reverse postorder visits a block after an incoming edge"
        );
        for statement in self.body.block_statements(block) {
            match statement {
                StatementKind::Assign(place, rvalue) => {
                    let ty = self.place_type(*place)?;
                    let value = self.semantic_rvalue(rvalue, ty, index, &mut control)?;
                    self.write_place(*place, value, index, &mut control)?;
                }
                StatementKind::Assume(predicate) => {
                    // Operand evaluation may load memory or deinitialize a
                    // moved place; the requirement follows those effects.
                    let predicate = self.operand(predicate, index, &mut control)?;
                    let effect = self.read_variable(self.effect, index);
                    let assume = self.graph.add_node(
                        Opcode::Assume,
                        control,
                        &[effect, predicate],
                        0,
                        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
                    );
                    control = self.projection(assume, 0, ValueType::Control);
                    let effect = self.projection(assume, 1, ValueType::Effect);
                    self.write_variable(self.effect, index, effect);
                }
                StatementKind::CopyNonOverlapping {
                    src,
                    dst,
                    count,
                    element_size,
                } => {
                    let source = self.operand(src, index, &mut control)?;
                    let destination = self.operand(dst, index, &mut control)?;
                    let count = self.operand(count, index, &mut control)?;
                    self.copy_nonoverlapping(
                        source,
                        destination,
                        count,
                        *element_size,
                        index,
                        &mut control,
                    )?;
                }
                StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                    if self.memory_locals[local.index()] {
                        let effect = self.read_variable(self.effect, index);
                        let opcode = if matches!(statement, StatementKind::StorageLive(_)) {
                            Opcode::SlotLive
                        } else {
                            Opcode::SlotDead
                        };
                        let node = self.graph.add_node(
                            opcode,
                            control,
                            &[effect, self.local_slots[local.index()]],
                            0,
                            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
                        );
                        control = self.projection(node, 0, ValueType::Control);
                        let effect = self.projection(node, 1, ValueType::Effect);
                        self.write_variable(self.effect, index, effect);
                    } else {
                        let value = self.undefined(local.0);
                        self.write_variable(local.0, index, value);
                    }
                }
                StatementKind::Nop => {}
                StatementKind::SetDiscriminant(place, variant) => {
                    self.set_discriminant(*place, variant.0, index, &mut control)?
                }
            }
        }
        let terminator = self.body.block(block).terminator.clone();
        self.terminator(&terminator, block, control)
    }

    fn terminator(
        &mut self,
        terminator: &Terminator,
        block: BlockId,
        mut control: NodeId,
    ) -> Result<(), BuildError> {
        let index = block.index();
        let source = EdgeSource::Block(block);
        match terminator {
            Terminator::Goto(target) => {
                self.add_edge(source, *target, control);
                Ok(())
            }
            Terminator::SwitchInt { discriminant, targets } => {
                let discriminant_ty = self.operand_type(discriminant);
                let value = self.operand(discriminant, index, &mut control)?;
                if self.variant_switch(value, discriminant_ty, targets, block, control) {
                    return Ok(());
                }
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
                // A source return always carries its Rust value. Ignore is a
                // physical transport decision made only at layout lowering.
                let ty = self.body.locals[LocalId::RETURN.index()].ty;
                let value = if matches!(
                    self.graph.oracle.shape(ty),
                    TypeShape::Unit | TypeShape::FunctionItem { .. }
                ) {
                    self.semantic_empty_value(ty)?
                } else {
                    self.read_place(Place::local(LocalId::RETURN), index, &mut control)?
                };
                let inputs = [self.read_variable(self.effect, index), value];
                self.graph.add_node(Opcode::Return, control, &inputs, 0, ValueType::Control);
                Ok(())
            }
            Terminator::Unreachable => {
                self.graph.add_node(Opcode::Unreachable, control, &[], 0, ValueType::Control);
                Ok(())
            }
            Terminator::Call { callee, args, destination, target, .. } =>
                self.build_call(*callee, None, args, *destination, *target, block, control),
            Terminator::CallIndirect { function, abi, args, destination, target, .. } =>
                self.build_call(*function, Some(abi), args, *destination, *target, block, control),
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
        self.graph.oracle.intern(
            TypeShape::Bool,
            Some(kfrc_body::SizeAlign { size: 1, align: 1 }),
        )
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

/// Address-taking affects a whole local, so every access to it observes the
/// same memory. Unborrowed scalars retain the original SSA representation.
fn memory_locals(body: &Body) -> Vec<bool> {
    let mut memory = vec![false; body.locals.len()];
    let mark_address = |place: Place, memory: &mut Vec<bool>| {
        if !body
            .projection(place.projection)
            .iter()
            .any(|p| matches!(p, ProjectionElem::Deref))
        {
            memory[place.local.index()] = true;
        }
    };
    let mark_index = |place: Place, memory: &mut Vec<bool>| {
        let mut ty = body.locals[place.local.index()].ty;
        let elements = body.projection(place.projection);
        for (position, element) in elements.iter().enumerate() {
            if matches!(element, ProjectionElem::Deref) {
                break;
            }
            if let ProjectionElem::Field(_, field_ty) = element {
                if body.types.semantic().maybe_uninit_inner(ty).is_some()
                    && position + 1 < elements.len()
                {
                    // A partial union-field update must preserve untouched
                    // raw bytes, not decode an uninitialized whole T first.
                    memory[place.local.index()] = true;
                    break;
                }
                ty = *field_ty;
            } else if matches!(element, ProjectionElem::ConstantIndex { .. }) {
                if let TypeShape::Array { element, .. } | TypeShape::Slice { element } =
                    body.types.shape(ty)
                {
                    ty = *element;
                }
            }
            if matches!(
                element,
                ProjectionElem::Index(_) | ProjectionElem::Subslice { .. }
            ) {
                memory[place.local.index()] = true;
                break;
            }
        }
    };
    let operand = |operand: &Operand, memory: &mut Vec<bool>| {
        if let Operand::Copy(place) | Operand::Move(place) = operand {
            mark_index(*place, memory);
        }
    };
    for statement in &body.statements {
        match statement {
            StatementKind::Assign(place, value) => {
                mark_index(*place, &mut memory);
                match value {
                    Rvalue::Ref(_, place) | Rvalue::RawPointer(_, place) => {
                        mark_address(*place, &mut memory)
                    }
                    Rvalue::Discriminant(place) | Rvalue::Len(place) => {
                        mark_index(*place, &mut memory)
                    }
                    Rvalue::Use(value)
                    | Rvalue::Opaque(value)
                    | Rvalue::Repeat(value, _)
                    | Rvalue::UnaryOp(_, value)
                    | Rvalue::Cast(_, value, _) => operand(value, &mut memory),
                    Rvalue::BinaryOp(_, left, right) => {
                        operand(left, &mut memory);
                        operand(right, &mut memory);
                    }
                    Rvalue::Aggregate(_, values) => {
                        for value in values {
                            operand(value, &mut memory);
                        }
                    }
                    _ => {}
                }
            }
            StatementKind::Assume(value) => operand(value, &mut memory),
            StatementKind::SetDiscriminant(place, _) => mark_address(*place, &mut memory),
            StatementKind::CopyNonOverlapping {
                src, dst, count, ..
            } => {
                operand(src, &mut memory);
                operand(dst, &mut memory);
                operand(count, &mut memory);
            }
            _ => {}
        }
    }
    for block in &body.blocks {
        match &block.terminator {
            Terminator::SwitchInt { discriminant, .. } => operand(discriminant, &mut memory),
            Terminator::Call {
                callee,
                args,
                destination,
                ..
            } => {
                operand(callee, &mut memory);
                for arg in args {
                    operand(arg, &mut memory);
                }
                mark_index(*destination, &mut memory);
            }
            Terminator::CallIndirect {
                function,
                args,
                destination,
                ..
            } => {
                operand(function, &mut memory);
                for arg in args {
                    operand(arg, &mut memory);
                }
                mark_index(*destination, &mut memory);
            }
            Terminator::Assert { condition, .. } => operand(condition, &mut memory),
            Terminator::Drop { place, .. } => mark_address(*place, &mut memory),
            _ => {}
        }
    }
    memory
}
