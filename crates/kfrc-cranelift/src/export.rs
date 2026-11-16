//! Scheduled Layout graph to Cranelift IR.
//!
//! One Cranelift block per scheduled block. Fixed nodes are emitted in chain
//! order, pure nodes at their first demand inside the block the schedule
//! placed them in, and region `Phi`s become block parameters. Cranelift's own
//! mid-end may move or merge the pure instructions afterwards.

mod abi;
mod intrinsics;
mod memory;
mod promote;
mod switch;

use cranelift_codegen::ir::condcodes::{CondCode, IntCC};
use cranelift_codegen::ir::{self, BlockArg, InstBuilder, TrapCode, Type, Value, types};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_frontend::FunctionBuilder;
use kfrc_body::fx_hash::HashMap;
use kfrc_body::{TyId, TypeOracle, TypeShape};
use kfrc_son::lower_layout::LayoutOptions;
use kfrc_son::schedule::Schedule;
use kfrc_son::{ArithmeticFlags, ComparePredicate, FunctionGraph, NodeId, Opcode, ValueType};

use crate::machine::MachineOptions;

#[derive(Debug)]
pub enum LowerError {
    NotVerified,
    Unsupported(String),
    Malformed(String),
    Codegen(String),
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotVerified => formatter.write_str("graph must be verified before lowering"),
            Self::Unsupported(what) => write!(formatter, "unsupported in machine lowering: {what}"),
            Self::Malformed(what) => write!(formatter, "malformed graph: {what}"),
            Self::Codegen(what) => write!(formatter, "Cranelift compilation failed: {what}"),
        }
    }
}

impl std::error::Error for LowerError {}

/// The integer representation of one Layout value. A zero-width scalar is a
/// value without bits (`()`, `!`, a function item) and has no Cranelift value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Scalar {
    pub bits: u8,
    pub signed: bool,
}

impl Scalar {
    pub(crate) const ADDRESS: Self = Self {
        bits: 64,
        signed: false,
    };

    pub(crate) fn ty(self) -> Option<Type> {
        integer_type(self.bits)
    }
}

pub(crate) fn integer_type(bits: u8) -> Option<Type> {
    match bits {
        8 => Some(types::I8),
        16 => Some(types::I16),
        32 => Some(types::I32),
        64 => Some(types::I64),
        _ => None,
    }
}

/// Operands and type of a multiplication whose product and overflow flag one
/// machine multiply can deliver together.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ProductKey {
    lhs: NodeId,
    rhs: NodeId,
    ty: TyId,
}

pub(crate) struct Exporter<'graph, 'function> {
    pub(crate) graph: &'graph FunctionGraph,
    pub(crate) schedule: &'graph Schedule,
    pub(crate) builder: FunctionBuilder<'function>,
    pub(crate) options: LayoutOptions,
    frontend: TargetFrontendConfig,
    blocks: Vec<ir::Block>,
    /// The Cranelift value of every emitted node, by node index.
    values: Vec<Option<Value>>,
    stack_slots: Vec<Option<ir::StackSlot>>,
    symbol_names: Vec<String>,
    symbol_index: HashMap<String, u32>,
    signatures: Vec<(ir::Signature, ir::SigRef)>,
    functions: HashMap<(u32, ir::SigRef), ir::FuncRef>,
    /// The caller's result storage of an indirectly returning function.
    pub(crate) return_pointer: Option<Value>,
    current_block: usize,
    /// Scheduled blocks that execution reaches only on the way to a trap.
    cold: Vec<bool>,
    /// Products already computed by an overflow check, with their block.
    products: HashMap<ProductKey, Vec<(usize, Value)>>,
    overflow_nodes: HashMap<(ProductKey, usize), NodeId>,
    /// Frame storage represented as SSA variables.
    promotion: promote::Promotion,
    machine: MachineOptions,
}

impl<'graph, 'function> Exporter<'graph, 'function> {
    pub(crate) fn new(
        graph: &'graph FunctionGraph,
        schedule: &'graph Schedule,
        builder: FunctionBuilder<'function>,
        options: LayoutOptions,
        frontend: TargetFrontendConfig,
        machine: MachineOptions,
    ) -> Self {
        Self {
            graph,
            schedule,
            builder,
            options,
            frontend,
            blocks: Vec::new(),
            values: vec![None; graph.len()],
            stack_slots: vec![None; graph.stack_slots.len()],
            symbol_names: Vec::new(),
            symbol_index: HashMap::default(),
            signatures: Vec::new(),
            functions: HashMap::default(),
            return_pointer: None,
            current_block: 0,
            cold: Vec::new(),
            products: HashMap::default(),
            overflow_nodes: HashMap::default(),
            promotion: promote::Promotion::default(),
            machine,
        }
    }

    /// Exports the whole function and returns its symbol table.
    pub(crate) fn run(mut self) -> Result<Vec<String>, LowerError> {
        let graph = self.graph;
        let schedule = self.schedule;
        self.blocks = (0..schedule.blocks.len())
            .map(|_| self.builder.create_block())
            .collect();
        self.builder.func.signature = self.function_signature()?;
        if self.machine.promote_storage {
            self.plan_promotion()?;
        }
        self.builder
            .append_block_params_for_function_params(self.blocks[0]);
        for (index, block) in schedule.blocks.iter().enumerate() {
            let Some(phis) = schedule.phis_of_region.get(&block.head) else {
                continue;
            };
            for &phi in phis {
                if let Some(ty) = self.phi_type(phi)? {
                    let value = self.builder.append_block_param(self.blocks[index], ty);
                    self.values[phi.index()] = Some(value);
                }
            }
        }
        self.cold = self.cold_blocks();
        // The entry block stays in place.
        self.cold[0] = false;
        for (index, cold) in self.cold.iter().enumerate() {
            if *cold {
                self.builder.set_cold_block(self.blocks[index]);
            }
        }
        for (block, nodes) in schedule.placed.iter().enumerate() {
            for &node in nodes {
                if graph.op(node) == Opcode::MulOverflow
                    && let Some(key) = self.product_key(node)
                {
                    self.overflow_nodes.entry((key, block)).or_insert(node);
                }
            }
        }
        for &block in &schedule.order {
            self.current_block = block;
            self.builder.switch_to_block(self.blocks[block]);
            if block == 0 {
                self.receive_parameters()?;
            }
            let (terminator, body) = schedule.blocks[block]
                .chain
                .split_last()
                .ok_or_else(|| LowerError::Malformed(format!("block {block} is empty")))?;
            for &node in body {
                self.emit_fixed(node)?;
            }
            // Values placed here and used by later blocks are defined before
            // control leaves.
            for &node in &schedule.placed[block] {
                // The address of promoted storage has no value.
                if self.values[node.index()].is_none()
                    && self.promotion.place(node).is_none()
                    && !self.promotion.returned(node)
                {
                    self.emit_pure(node)?;
                }
            }
            self.emit_terminator(*terminator, block)?;
        }
        self.builder.seal_all_blocks();
        self.builder.finalize(self.frontend);
        Ok(self.symbol_names)
    }

    /// Continues the current scheduled block in a fresh Cranelift block.
    ///
    /// Cranelift produces a constant once per block and again in every other
    /// block that uses it. A boundary after each call therefore re-creates a
    /// constant behind the call instead of carrying it across in a
    /// callee-saved register; the fallthrough jump has no machine code.
    pub(crate) fn continue_in_new_block(&mut self) {
        if !self.machine.split_after_calls {
            return;
        }
        let next = self.builder.create_block();
        self.builder.ins().jump(next, &[]);
        if self.cold[self.current_block] {
            self.builder.set_cold_block(next);
        }
        self.builder.switch_to_block(next);
    }

    /// Blocks that end in `Unreachable` (after a diverging call or behind
    /// undefined behaviour), and blocks all of whose successors are cold.
    fn cold_blocks(&self) -> Vec<bool> {
        let blocks = &self.schedule.blocks;
        let mut cold: Vec<bool> = blocks
            .iter()
            .map(|block| {
                block
                    .chain
                    .last()
                    .is_some_and(|&last| self.graph.op(last) == Opcode::Unreachable)
            })
            .collect();
        loop {
            let mut changed = false;
            for (index, block) in blocks.iter().enumerate() {
                if !cold[index]
                    && !block.successors.is_empty()
                    && block.successors.iter().all(|&successor| cold[successor])
                {
                    cold[index] = true;
                    changed = true;
                }
            }
            if !changed {
                return cold;
            }
        }
    }

    pub(crate) fn scalar_of_type(&self, ty: TyId) -> Result<Scalar, LowerError> {
        let oracle = &self.graph.oracle;
        match oracle.shape(ty) {
            TypeShape::Bool => Ok(Scalar {
                bits: 8,
                signed: false,
            }),
            TypeShape::Char => Ok(Scalar {
                bits: 32,
                signed: false,
            }),
            TypeShape::Int { width, signed } => {
                let bits = width.bits(64);
                if bits > 64 {
                    return Err(LowerError::Unsupported(format!("{bits}-bit integers")));
                }
                Ok(Scalar {
                    bits,
                    signed: *signed,
                })
            }
            TypeShape::PlaceAddress { .. }
            | TypeShape::Reference { .. }
            | TypeShape::RawPointer { .. }
                if oracle.semantic().thin_pointee(ty).is_some()
                    && oracle.size_align(ty).is_some_and(|layout| layout.size == 8) =>
            {
                Ok(Scalar::ADDRESS)
            }
            TypeShape::Unit | TypeShape::Never | TypeShape::FunctionItem { .. } => Ok(Scalar {
                bits: 0,
                signed: false,
            }),
            _ => Err(LowerError::Unsupported(format!(
                "values of type `{}`",
                oracle.describe(ty)
            ))),
        }
    }

    /// The representation of a value node. An aggregate at Layout level is
    /// the address of its storage.
    pub(crate) fn scalar_of_node(&self, node: NodeId) -> Result<Scalar, LowerError> {
        let node = self.graph.resolve(node);
        if matches!(
            self.graph.op(node),
            Opcode::StackSlot | Opcode::SymbolAddress | Opcode::PtrOffset
        ) {
            return Ok(Scalar::ADDRESS);
        }
        match self.graph.value_type(node) {
            ValueType::LayoutValue(_) => Ok(Scalar::ADDRESS),
            ValueType::Rust(ty) => self.scalar_of_type(*ty),
            _ => Err(LowerError::Malformed(format!("{node} is not a value"))),
        }
    }

    fn is_boolean(&self, node: NodeId) -> bool {
        matches!(
            self.graph.value_type(self.graph.resolve(node)),
            ValueType::Rust(ty) if matches!(self.graph.oracle.shape(*ty), TypeShape::Bool)
        )
    }

    /// The block parameter type of a region phi. Effects, values without
    /// bits and merged storage (whose lanes are variables) have none.
    fn phi_type(&self, phi: NodeId) -> Result<Option<Type>, LowerError> {
        if matches!(self.graph.value_type(phi), ValueType::Effect)
            || self.promotion.merged(phi).is_some()
            || self.promotion.returned(phi)
        {
            return Ok(None);
        }
        Ok(self.scalar_of_node(phi)?.ty())
    }

    pub(crate) fn define(&mut self, node: NodeId, value: Value) {
        self.values[node.index()] = Some(value);
    }

    pub(crate) fn projection(&self, node: NodeId, index: u32) -> Option<NodeId> {
        self.schedule.projections.get(&(node, index)).copied()
    }

    /// A constant of `ty` from the low bits of `bits`.
    pub(crate) fn integer_constant(&mut self, ty: Type, bits: u64) -> Value {
        let masked = if ty.bits() >= 64 {
            bits
        } else {
            bits & ((1u64 << ty.bits()) - 1)
        };
        self.builder.ins().iconst(ty, masked as i64)
    }

    /// Converts between integer widths: extension follows the signedness of
    /// the source representation, narrowing keeps the low bits.
    pub(crate) fn coerce(&mut self, value: Value, from: Scalar, bits: u8) -> Value {
        let Some(target) = integer_type(bits) else {
            return value;
        };
        let source = self.builder.func.dfg.value_type(value);
        if source == target {
            value
        } else if source.bits() < target.bits() {
            if from.signed {
                self.builder.ins().sextend(target, value)
            } else {
                self.builder.ins().uextend(target, value)
            }
        } else {
            self.builder.ins().ireduce(target, value)
        }
    }

    /// The value of `node` in an integer of `bits` bits.
    pub(crate) fn operand(&mut self, node: NodeId, bits: u8) -> Result<Value, LowerError> {
        let scalar = self.scalar_of_node(node)?;
        let value = self.value(node)?;
        Ok(self.coerce(value, scalar, bits))
    }

    /// The value of an address-typed node. Pointer types the scalar table does
    /// not describe (function pointers) are still one 64-bit register.
    pub(crate) fn address(&mut self, node: NodeId) -> Result<Value, LowerError> {
        let value = self.value(node)?;
        Ok(self.coerce(value, Scalar::ADDRESS, 64))
    }

    /// The Cranelift value of a node, emitting a pure node at first demand.
    /// A value without bits reads as zero.
    pub(crate) fn value(&mut self, node: NodeId) -> Result<Value, LowerError> {
        let node = self.graph.resolve(node);
        if let Some(value) = self.values[node.index()] {
            return Ok(value);
        }
        if self.promotion.place(node).is_some() || self.promotion.returned(node) {
            return Err(LowerError::Malformed(format!(
                "the address {node} of promoted storage is used as a value"
            )));
        }
        let op = self.graph.op(node);
        match op {
            Opcode::Const => {
                let scalar = self.scalar_of_node(node)?;
                let bits = self.graph.constant_value(node) as u64;
                Ok(match scalar.ty() {
                    Some(ty) => self.integer_constant(ty, bits),
                    None => self.builder.ins().iconst(types::I64, 0),
                })
            }
            Opcode::Undefined => {
                let ty = self
                    .scalar_of_node(node)
                    .ok()
                    .and_then(Scalar::ty)
                    .unwrap_or(types::I64);
                Ok(self.builder.ins().iconst(ty, 0))
            }
            _ if matches!(op, Opcode::Phi | Opcode::Projection) || op.is_fixed() => {
                if self.scalar_of_node(node)?.bits == 0 {
                    Ok(self.builder.ins().iconst(types::I64, 0))
                } else {
                    Err(LowerError::Malformed(format!(
                        "{node} used before its definition was scheduled"
                    )))
                }
            }
            _ => self.emit_pure(node),
        }
    }

    fn product_key(&self, node: NodeId) -> Option<ProductKey> {
        let graph = self.graph;
        if !matches!(graph.op(node), Opcode::Mul | Opcode::MulOverflow) {
            return None;
        }
        let mut lhs = graph.resolve(graph.input(node, 0));
        let mut rhs = graph.resolve(graph.input(node, 1));
        let ValueType::Rust(ty) = *graph.value_type(lhs) else {
            return None;
        };
        if *graph.value_type(rhs) != ValueType::Rust(ty)
            || !matches!(graph.oracle.shape(ty), TypeShape::Int { .. })
        {
            return None;
        }
        if graph.op(node) == Opcode::Mul && *graph.value_type(node) != ValueType::Rust(ty) {
            return None;
        }
        if lhs.index() > rhs.index() {
            std::mem::swap(&mut lhs, &mut rhs);
        }
        Some(ProductKey { lhs, rhs, ty })
    }

    fn emit_pure(&mut self, node: NodeId) -> Result<Value, LowerError> {
        let graph = self.graph;
        let op = graph.op(node);
        let value = match op {
            Opcode::StackSlot => self.stack_slot_address(graph.payload(node) as usize)?,
            Opcode::SymbolAddress => {
                let (symbol, addend) = &graph.symbols[graph.payload(node) as usize];
                self.symbol_address(symbol, *addend, false)
            }
            Opcode::PtrOffset => {
                let base = self.address(graph.input(node, 0))?;
                let offset_node = graph.resolve(graph.input(node, 1));
                if graph.op(offset_node) == Opcode::Const && graph.constant_value(offset_node) == 0
                {
                    base
                } else {
                    let offset = self.operand(offset_node, 64)?;
                    self.builder.ins().iadd(base, offset)
                }
            }
            Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor => {
                let scalar = self.scalar_of_node(node)?;
                if op == Opcode::Mul
                    && let Some(product) = self.shared_product(node)?
                {
                    self.define(node, product);
                    return Ok(product);
                }
                let lhs = self.operand(graph.input(node, 0), scalar.bits)?;
                let rhs = self.operand(graph.input(node, 1), scalar.bits)?;
                let ins = self.builder.ins();
                match op {
                    Opcode::Add => ins.iadd(lhs, rhs),
                    Opcode::Sub => ins.isub(lhs, rhs),
                    Opcode::Mul => ins.imul(lhs, rhs),
                    Opcode::BitAnd => ins.band(lhs, rhs),
                    Opcode::BitOr => ins.bor(lhs, rhs),
                    _ => ins.bxor(lhs, rhs),
                }
            }
            Opcode::ShiftLeft | Opcode::ShiftRightArithmetic | Opcode::ShiftRightLogical => {
                // Cranelift takes the amount modulo the width of the shifted
                // value, the graph's own definition of an ordinary shift.
                let scalar = self.scalar_of_node(node)?;
                let lhs = self.operand(graph.input(node, 0), scalar.bits)?;
                let rhs = self.value(graph.input(node, 1))?;
                let ins = self.builder.ins();
                match op {
                    Opcode::ShiftLeft => ins.ishl(lhs, rhs),
                    Opcode::ShiftRightArithmetic => ins.sshr(lhs, rhs),
                    _ => ins.ushr(lhs, rhs),
                }
            }
            Opcode::Negate => {
                let scalar = self.scalar_of_node(node)?;
                let operand = self.operand(graph.input(node, 0), scalar.bits)?;
                self.builder.ins().ineg(operand)
            }
            Opcode::BitNot => {
                let scalar = self.scalar_of_node(node)?;
                let input = graph.resolve(graph.input(node, 0));
                if !self.is_boolean(input) {
                    let operand = self.operand(input, scalar.bits)?;
                    self.builder.ins().bnot(operand)
                } else if graph.op(input) == Opcode::Compare {
                    // The negation of a comparison is the complementary
                    // comparison of the same operands.
                    self.emit_compare(input, true)?
                } else {
                    let operand = self.operand(input, scalar.bits)?;
                    let one = self.builder.ins().iconst(types::I8, 1);
                    self.builder.ins().bxor(operand, one)
                }
            }
            Opcode::IntCast => {
                let scalar = self.scalar_of_node(node)?;
                if scalar.bits == 0 {
                    self.builder.ins().iconst(types::I64, 0)
                } else {
                    self.operand(graph.input(node, 0), scalar.bits)?
                }
            }
            Opcode::Compare => self.emit_compare(node, false)?,
            Opcode::MulOverflow => {
                let scalar = self.scalar_of_node(graph.input(node, 0))?;
                let lhs = self.operand(graph.input(node, 0), scalar.bits)?;
                let rhs = self.operand(graph.input(node, 1), scalar.bits)?;
                let (product, overflow) = if scalar.signed {
                    self.builder.ins().smul_overflow(lhs, rhs)
                } else {
                    self.builder.ins().umul_overflow(lhs, rhs)
                };
                if let Some(key) = self.product_key(node) {
                    self.products
                        .entry(key)
                        .or_default()
                        .push((self.current_block, product));
                }
                overflow
            }
            Opcode::BitCount
            | Opcode::ByteSwap
            | Opcode::BitReverse
            | Opcode::FunnelShift
            | Opcode::SaturatingAdd
            | Opcode::SaturatingSub => self.emit_integer_intrinsic(node)?,
            other => {
                return Err(LowerError::Unsupported(format!(
                    "{} node {node}",
                    other.name()
                )));
            }
        };
        self.define(node, value);
        Ok(value)
    }

    /// The comparison `node`, or its complement.
    fn emit_compare(&mut self, node: NodeId, complement: bool) -> Result<Value, LowerError> {
        let graph = self.graph;
        let (lhs, rhs) = (graph.input(node, 0), graph.input(node, 1));
        let bits = self
            .scalar_of_node(lhs)?
            .bits
            .max(self.scalar_of_node(rhs)?.bits)
            .max(8);
        let lhs = self.operand(lhs, bits)?;
        let rhs = self.operand(rhs, bits)?;
        let condition = condition(ComparePredicate::from_payload(graph.payload(node)));
        let condition = if complement {
            condition.complement()
        } else {
            condition
        };
        Ok(self.builder.ins().icmp(condition, lhs, rhs))
    }

    /// The low half of a multiplication that an overflow check on the same
    /// operands computes in a dominating position.
    fn shared_product(&mut self, node: NodeId) -> Result<Option<Value>, LowerError> {
        if ArithmeticFlags(self.graph.payload(node)).0
            & !(ArithmeticFlags::NUW | ArithmeticFlags::NSW)
            != 0
        {
            return Ok(None);
        }
        let Some(key) = self.product_key(node) else {
            return Ok(None);
        };
        if let Some(&overflow) = self.overflow_nodes.get(&(key, self.current_block))
            && self.values[overflow.index()].is_none()
        {
            self.emit_pure(overflow)?;
        }
        Ok(self.products.get(&key).and_then(|definitions| {
            definitions.iter().rev().find_map(|&(block, product)| {
                self.schedule
                    .dominates(block, self.current_block)
                    .then_some(product)
            })
        }))
    }

    fn emit_fixed(&mut self, node: NodeId) -> Result<(), LowerError> {
        let graph = self.graph;
        match graph.op(node) {
            op @ (Opcode::DivSigned
            | Opcode::DivUnsigned
            | Opcode::RemSigned
            | Opcode::RemUnsigned) => {
                let scalar = self.scalar_of_node(node)?;
                let lhs = self.operand(graph.input(node, 0), scalar.bits)?;
                let rhs = self.operand(graph.input(node, 1), scalar.bits)?;
                let ins = self.builder.ins();
                let value = match op {
                    Opcode::DivSigned => ins.sdiv(lhs, rhs),
                    Opcode::DivUnsigned => ins.udiv(lhs, rhs),
                    Opcode::RemSigned => ins.srem(lhs, rhs),
                    _ => ins.urem(lhs, rhs),
                };
                self.define(node, value);
            }
            // Lifetime markers carry no code at this layer.
            Opcode::SlotLive | Opcode::SlotDead => {}
            Opcode::Opaque => {
                if let Some(result) = self.projection(node, 2) {
                    let value = self.value(graph.input(node, 1))?;
                    self.define(result, value);
                }
            }
            Opcode::ThreadLocalAddress => {
                if let Some(result) = self.projection(node, 2) {
                    let (symbol, addend) = &graph.symbols[graph.payload(node) as usize];
                    if *addend != 0 {
                        return Err(LowerError::Malformed("TLS symbol has an addend".into()));
                    }
                    let value = self.symbol_address(symbol, 0, true);
                    self.define(result, value);
                }
            }
            Opcode::Load => self.emit_load(node)?,
            Opcode::Store => self.emit_store(node)?,
            Opcode::Call | Opcode::CallIndirect => self.emit_call(node)?,
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
            Opcode::Switch => self.emit_switch(node, block)?,
            Opcode::If => {
                // A defined `bool` is exactly 0 or 1: each negation between
                // the branch and its source swaps the two successors.
                let mut source = graph.resolve(graph.input(node, 0));
                let mut negated = false;
                while graph.op(source) == Opcode::BitNot && self.is_boolean(graph.input(source, 0))
                {
                    source = graph.resolve(graph.input(source, 0));
                    negated = !negated;
                }
                let condition = self.value(source)?;
                let successors = &schedule.blocks[block].successors;
                let (mut then_block, mut else_block) =
                    (self.blocks[successors[0]], self.blocks[successors[1]]);
                if negated {
                    std::mem::swap(&mut then_block, &mut else_block);
                }
                self.builder
                    .ins()
                    .brif(condition, then_block, &[], else_block, &[]);
            }
            Opcode::Jump => {
                let (region, slot) = schedule.region_of_jump[&node];
                let mut arguments = Vec::new();
                // Lanes of merged storage are assigned together after every
                // source lane of this edge has been read.
                let mut assignments = Vec::new();
                if let Some(phis) = schedule.phis_of_region.get(&region) {
                    for &phi in phis {
                        if self.promotion.returned(phi) {
                            let input = graph.resolve(graph.input(phi, slot + 1));
                            if graph.op(input) != Opcode::Undefined {
                                self.transfer_to_caller(input)?;
                            }
                            continue;
                        }
                        if let Some(storage) = self.promotion.merged(phi) {
                            let input = graph.resolve(graph.input(phi, slot + 1));
                            if graph.op(input) != Opcode::Undefined {
                                for lane in self.promotion.lanes(storage).to_vec() {
                                    let value = self.read_place(input, lane.offset, lane.ty())?;
                                    assignments.push((lane.variable, value));
                                }
                            }
                            continue;
                        }
                        let Some(ty) = self.phi_type(phi)? else {
                            continue;
                        };
                        let input = graph.resolve(graph.input(phi, slot + 1));
                        let value = if graph.op(input) == Opcode::Undefined {
                            self.builder.ins().iconst(ty, 0)
                        } else {
                            self.operand(input, ty.bits() as u8)?
                        };
                        arguments.push(BlockArg::Value(value));
                    }
                }
                for (variable, value) in assignments {
                    self.builder.def_var(variable, value);
                }
                let target = self.blocks[schedule.block_of_head[&region]];
                self.builder.ins().jump(target, &arguments);
            }
            Opcode::Return => self.emit_return(node)?,
            // Reached only through undefined behaviour or after a diverging
            // call.
            Opcode::Unreachable => {
                self.builder.ins().trap(TrapCode::unwrap_user(1));
            }
            other => {
                return Err(LowerError::Malformed(format!(
                    "{} cannot end a block",
                    other.name()
                )));
            }
        }
        Ok(())
    }

    /// The index of `name` in the function's symbol table.
    fn symbol(&mut self, name: &str) -> u32 {
        if let Some(&index) = self.symbol_index.get(name) {
            return index;
        }
        let index = self.symbol_names.len() as u32;
        self.symbol_names.push(name.to_owned());
        self.symbol_index.insert(name.to_owned(), index);
        index
    }

    fn external_name(&mut self, name: &str) -> ir::ExternalName {
        let index = self.symbol(name);
        let reference = self
            .builder
            .func
            .declare_imported_user_function(ir::UserExternalName {
                namespace: 0,
                index,
            });
        ir::ExternalName::user(reference)
    }

    /// The address of a linker symbol, or of the current thread's instance of
    /// a thread-local symbol.
    pub(crate) fn symbol_address(&mut self, name: &str, addend: i64, thread_local: bool) -> Value {
        let name = self.external_name(name);
        let global = self
            .builder
            .create_global_value(ir::GlobalValueData::Symbol {
                name,
                offset: ir::immediates::Imm64::new(addend),
                colocated: true,
                tls: thread_local,
            });
        if thread_local {
            self.builder.ins().tls_value(types::I64, global)
        } else {
            self.builder.ins().symbol_value(types::I64, global)
        }
    }

    pub(crate) fn signature_reference(&mut self, signature: ir::Signature) -> ir::SigRef {
        if let Some((_, reference)) = self
            .signatures
            .iter()
            .find(|(existing, _)| *existing == signature)
        {
            return *reference;
        }
        let reference = self.builder.import_signature(signature.clone());
        self.signatures.push((signature, reference));
        reference
    }

    pub(crate) fn function_reference(&mut self, name: &str, signature: ir::SigRef) -> ir::FuncRef {
        let index = self.symbol(name);
        if let Some(&reference) = self.functions.get(&(index, signature)) {
            return reference;
        }
        let name = self.external_name(name);
        let reference = self.builder.import_function(ir::ExtFuncData {
            name,
            signature,
            colocated: true,
            patchable: false,
        });
        self.functions.insert((index, signature), reference);
        reference
    }
}

fn condition(predicate: ComparePredicate) -> IntCC {
    match predicate {
        ComparePredicate::Equal => IntCC::Equal,
        ComparePredicate::NotEqual => IntCC::NotEqual,
        ComparePredicate::SignedLess => IntCC::SignedLessThan,
        ComparePredicate::SignedLessEqual => IntCC::SignedLessThanOrEqual,
        ComparePredicate::SignedGreater => IntCC::SignedGreaterThan,
        ComparePredicate::SignedGreaterEqual => IntCC::SignedGreaterThanOrEqual,
        ComparePredicate::UnsignedLess => IntCC::UnsignedLessThan,
        ComparePredicate::UnsignedLessEqual => IntCC::UnsignedLessThanOrEqual,
        ComparePredicate::UnsignedGreater => IntCC::UnsignedGreaterThan,
        ComparePredicate::UnsignedGreaterEqual => IntCC::UnsignedGreaterThanOrEqual,
    }
}
