//! Function graph storage (`docs/design/ir/graph.md`).
//!
//! Nodes are 16-byte records in creation order with two side columns
//! (control input and result type). Nodes with more than two inputs keep them
//! in a shared pool. There are no use lists: a replaced node forwards to its
//! replacement, and `compact` rewrites inputs through the forwarding table
//! and drops dead nodes, preserving creation order.

use std::fmt::{self, Write as _};

use kfrc_body::{RecordedTypes, TyId, TypeOracle};

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

impl NodeId {
    pub const NONE: Self = Self(u32::MAX);

    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "n{}", self.0)
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Opcode {
    /// Produces `(Control, Effect, parameters…)`; read through projections.
    Start,
    /// `payload` selects a result of a multi-result node.
    Projection,
    /// Control merge; inputs are the predecessor `Jump`s, in order.
    Region,
    /// One control edge into a `Region`.
    Jump,
    /// Two-way branch on a `bool`; projection 0 is true, 1 is false.
    If,
    /// Inputs: the region, then one value per region predecessor.
    Phi,
    /// Inputs: effect, then the return value (absent for ignored returns).
    Return,
    /// Control reaches here only through undefined behaviour or divergence.
    Unreachable,
    /// `payload` indexes the callee table; inputs: effect, arguments.
    /// Produces `(Control, Effect, Value)`.
    Call,
    /// `payload` indexes the constant pool.
    Const,
    /// A value read before any definition on some path (poison).
    Undefined,
    Add,
    Sub,
    Mul,
    /// Division and remainder are fixed nodes (they may trap).
    DivSigned,
    DivUnsigned,
    RemSigned,
    RemUnsigned,
    BitAnd,
    BitOr,
    BitXor,
    /// Shift amount taken modulo the bit width of the shifted value.
    ShiftLeft,
    ShiftRightArithmetic,
    ShiftRightLogical,
    Negate,
    BitNot,
    /// `payload` is a `ComparePredicate`.
    Compare,
    /// Integer to integer conversion (truncate or extend by the types).
    IntCast,
}

impl Opcode {
    pub fn is_binary_arithmetic(self) -> bool {
        matches!(
            self,
            Self::Add
                | Self::Sub
                | Self::Mul
                | Self::DivSigned
                | Self::DivUnsigned
                | Self::RemSigned
                | Self::RemUnsigned
                | Self::BitAnd
                | Self::BitOr
                | Self::BitXor
                | Self::ShiftLeft
                | Self::ShiftRightArithmetic
                | Self::ShiftRightLogical
        )
    }

    /// Fixed nodes sit on the control skeleton.
    pub fn is_fixed(self) -> bool {
        matches!(
            self,
            Self::Start
                | Self::Region
                | Self::Jump
                | Self::If
                | Self::Return
                | Self::Unreachable
                | Self::Call
                | Self::DivSigned
                | Self::DivUnsigned
                | Self::RemSigned
                | Self::RemUnsigned
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Projection => "Projection",
            Self::Region => "Region",
            Self::Jump => "Jump",
            Self::If => "If",
            Self::Phi => "Phi",
            Self::Return => "Return",
            Self::Unreachable => "Unreachable",
            Self::Call => "Call",
            Self::Const => "Const",
            Self::Undefined => "Undefined",
            Self::Add => "Add.wrap",
            Self::Sub => "Sub.wrap",
            Self::Mul => "Mul.wrap",
            Self::DivSigned => "Div.signed",
            Self::DivUnsigned => "Div.unsigned",
            Self::RemSigned => "Rem.signed",
            Self::RemUnsigned => "Rem.unsigned",
            Self::BitAnd => "BitAnd",
            Self::BitOr => "BitOr",
            Self::BitXor => "BitXor",
            Self::ShiftLeft => "Shl.mask",
            Self::ShiftRightArithmetic => "Shr.arithmetic.mask",
            Self::ShiftRightLogical => "Shr.logical.mask",
            Self::Negate => "Neg.wrap",
            Self::BitNot => "Not",
            Self::Compare => "Compare",
            Self::IntCast => "Cast.IntToInt",
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComparePredicate {
    Equal,
    NotEqual,
    SignedLess,
    SignedLessEqual,
    SignedGreater,
    SignedGreaterEqual,
    UnsignedLess,
    UnsignedLessEqual,
    UnsignedGreater,
    UnsignedGreaterEqual,
}

impl ComparePredicate {
    pub const ALL: [Self; 10] = [
        Self::Equal,
        Self::NotEqual,
        Self::SignedLess,
        Self::SignedLessEqual,
        Self::SignedGreater,
        Self::SignedGreaterEqual,
        Self::UnsignedLess,
        Self::UnsignedLessEqual,
        Self::UnsignedGreater,
        Self::UnsignedGreaterEqual,
    ];

    pub fn from_payload(payload: u32) -> Self {
        Self::ALL[payload as usize]
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Equal => "eq",
            Self::NotEqual => "ne",
            Self::SignedLess => "slt",
            Self::SignedLessEqual => "sle",
            Self::SignedGreater => "sgt",
            Self::SignedGreaterEqual => "sge",
            Self::UnsignedLess => "ult",
            Self::UnsignedLessEqual => "ule",
            Self::UnsignedGreater => "ugt",
            Self::UnsignedGreaterEqual => "uge",
        }
    }
}

/// Inputs live in the extra pool: `in0` is the start, `in1` the count.
const FLAG_POOLED_INPUTS: u16 = 1;
const FLAG_DEAD: u16 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeRecord {
    op: Opcode,
    flags: u16,
    payload: u32,
    in0: NodeId,
    in1: NodeId,
}

const _: () = assert!(std::mem::size_of::<NodeRecord>() == 16);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ValueType {
    Rust(TyId),
    Control,
    Effect,
    Tuple(Vec<ValueType>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ValueTypeId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphLevel {
    Rust,
    Specialized,
    Layout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphState {
    Building,
    Sealed,
    Verified,
}

/// A direct call target: a function instance by linker symbol.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallTarget {
    pub symbol: String,
    pub abi: kfrc_body::FunctionAbi,
}

#[derive(Clone, Debug)]
pub struct FunctionGraph {
    pub symbol: String,
    pub abi: kfrc_body::FunctionAbi,
    records: Vec<NodeRecord>,
    control: Vec<NodeId>,
    types: Vec<ValueTypeId>,
    extra: Vec<NodeId>,
    forward: Vec<NodeId>,
    value_types: Vec<ValueType>,
    pub constants: Vec<u128>,
    pub callees: Vec<CallTarget>,
    pub start: NodeId,
    pub state: GraphState,
    pub level: GraphLevel,
    /// Answers about Rust types (P0: recorded by the Body producer).
    pub oracle: RecordedTypes,
}

impl FunctionGraph {
    pub fn new(symbol: impl Into<String>, abi: kfrc_body::FunctionAbi, oracle: RecordedTypes) -> Self {
        Self {
            symbol: symbol.into(),
            abi,
            records: Vec::new(),
            control: Vec::new(),
            types: Vec::new(),
            extra: Vec::new(),
            forward: Vec::new(),
            value_types: Vec::new(),
            constants: Vec::new(),
            callees: Vec::new(),
            start: NodeId::NONE,
            state: GraphState::Building,
            level: GraphLevel::Rust,
            oracle,
        }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn intern_type(&mut self, ty: ValueType) -> ValueTypeId {
        if let Some(index) = self.value_types.iter().position(|existing| *existing == ty) {
            return ValueTypeId(index as u32);
        }
        self.value_types.push(ty);
        ValueTypeId(self.value_types.len() as u32 - 1)
    }

    pub fn add_node(&mut self, op: Opcode, control: NodeId, inputs: &[NodeId], payload: u32, ty: ValueType) -> NodeId {
        let id = NodeId(self.records.len() as u32);
        let (flags, in0, in1) = if inputs.len() <= 2 {
            (
                0,
                inputs.first().copied().unwrap_or(NodeId::NONE),
                inputs.get(1).copied().unwrap_or(NodeId::NONE),
            )
        } else {
            let start = self.extra.len() as u32;
            self.extra.extend_from_slice(inputs);
            (FLAG_POOLED_INPUTS, NodeId(start), NodeId(inputs.len() as u32))
        };
        // Up to two inputs inline; a node that later grows (Region, Phi)
        // must be created pooled, see `add_growable_node`.
        let ty = self.intern_type(ty);
        self.records.push(NodeRecord {
            op,
            flags,
            payload,
            in0,
            in1,
        });
        self.control.push(control);
        self.types.push(ty);
        self.forward.push(NodeId::NONE);
        id
    }

    /// A node whose inputs are filled later (`Region` predecessors, `Phi`
    /// operands): `count` pooled slots initialized to `NONE`.
    pub fn add_growable_node(&mut self, op: Opcode, control: NodeId, count: usize, payload: u32, ty: ValueType) -> NodeId {
        let id = NodeId(self.records.len() as u32);
        let start = self.extra.len() as u32;
        self.extra.extend(std::iter::repeat_n(NodeId::NONE, count));
        let ty = self.intern_type(ty);
        self.records.push(NodeRecord {
            op,
            flags: FLAG_POOLED_INPUTS,
            payload,
            in0: NodeId(start),
            in1: NodeId(count as u32),
        });
        self.control.push(control);
        self.types.push(ty);
        self.forward.push(NodeId::NONE);
        id
    }

    pub fn op(&self, node: NodeId) -> Opcode {
        self.records[node.index()].op
    }

    pub fn payload(&self, node: NodeId) -> u32 {
        self.records[node.index()].payload
    }

    pub fn control(&self, node: NodeId) -> NodeId {
        self.control[node.index()]
    }

    pub fn value_type(&self, node: NodeId) -> &ValueType {
        &self.value_types[self.types[node.index()].0 as usize]
    }

    pub fn rust_type(&self, node: NodeId) -> Option<TyId> {
        match self.value_type(node) {
            ValueType::Rust(ty) => Some(*ty),
            _ => None,
        }
    }

    pub fn is_dead(&self, node: NodeId) -> bool {
        self.records[node.index()].flags & FLAG_DEAD != 0
    }

    pub fn inputs(&self, node: NodeId) -> &[NodeId] {
        let record = &self.records[node.index()];
        if record.flags & FLAG_POOLED_INPUTS != 0 {
            let start = record.in0.index();
            &self.extra[start..start + record.in1.index()]
        } else if record.in0.is_none() {
            &[]
        } else if record.in1.is_none() {
            std::slice::from_ref(&self.records[node.index()].in0)
        } else {
            let base = std::ptr::from_ref(&self.records[node.index()]);
            // SAFETY: `NodeRecord` is `repr(C)`; `in0` and `in1` are adjacent
            // `repr(transparent)` `u32` wrappers at offsets 8 and 12, and the
            // pointer is derived from the whole record, so the two-element
            // slice stays inside it and is borrowed for `&self`.
            unsafe { std::slice::from_raw_parts(std::ptr::addr_of!((*base).in0), 2) }
        }
    }

    pub fn input(&self, node: NodeId, index: usize) -> NodeId {
        self.inputs(node)[index]
    }

    pub fn set_input(&mut self, node: NodeId, index: usize, value: NodeId) {
        let record = self.records[node.index()];
        if record.flags & FLAG_POOLED_INPUTS != 0 {
            assert!(index < record.in1.index(), "pooled input index out of range");
            self.extra[record.in0.index() + index] = value;
        } else {
            match index {
                0 => self.records[node.index()].in0 = value,
                1 => self.records[node.index()].in1 = value,
                _ => panic!("inline nodes have at most two inputs"),
            }
        }
    }

    pub fn set_control(&mut self, node: NodeId, control: NodeId) {
        self.control[node.index()] = control;
    }

    pub fn constant_value(&self, node: NodeId) -> u128 {
        self.constants[self.payload(node) as usize]
    }

    pub fn add_constant(&mut self, bits: u128, ty: TyId) -> NodeId {
        let index = match self.constants.iter().position(|existing| *existing == bits) {
            Some(index) => index,
            None => {
                self.constants.push(bits);
                self.constants.len() - 1
            }
        };
        self.add_node(Opcode::Const, NodeId::NONE, &[], index as u32, ValueType::Rust(ty))
    }

    pub fn add_callee(&mut self, target: CallTarget) -> u32 {
        if let Some(index) = self.callees.iter().position(|existing| *existing == target) {
            return index as u32;
        }
        self.callees.push(target);
        self.callees.len() as u32 - 1
    }

    /// `node` is replaced by `replacement` everywhere (applied by `resolve`
    /// and `compact`).
    pub fn replace(&mut self, node: NodeId, replacement: NodeId) {
        let replacement = self.resolve(replacement);
        if node != replacement {
            self.forward[node.index()] = replacement;
            self.records[node.index()].flags |= FLAG_DEAD;
        }
    }

    pub fn resolve(&self, mut node: NodeId) -> NodeId {
        while !node.is_none() {
            let next = self.forward[node.index()];
            if next.is_none() {
                break;
            }
            node = next;
        }
        node
    }

    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        (0..self.records.len() as u32).map(NodeId).filter(|node| !self.is_dead(*node))
    }

    /// Rewrites every input through the forwarding table, removes nodes not
    /// reachable from the control skeleton, and renumbers in creation order.
    pub fn compact(&mut self) {
        let count = self.records.len();
        // Resolve forwarding in all inputs and control edges.
        for index in 0..count {
            let node = NodeId(index as u32);
            if self.is_dead(node) {
                continue;
            }
            let inputs: Vec<NodeId> = self.inputs(node).to_vec();
            for (slot, input) in inputs.into_iter().enumerate() {
                let resolved = self.resolve(input);
                if resolved != input {
                    self.set_input(node, slot, resolved);
                }
            }
            let control = self.control[index];
            self.control[index] = self.resolve(control);
        }
        // Mark live: roots are every live node that ends or carries control
        // (Return, Unreachable, Jump, If, Call) and their transitive inputs.
        let mut live = vec![false; count];
        let mut stack: Vec<NodeId> = self
            .ids()
            .filter(|node| {
                matches!(
                    self.op(*node),
                    Opcode::Start | Opcode::Return | Opcode::Unreachable | Opcode::Jump | Opcode::If | Opcode::Call
                )
            })
            .collect();
        while let Some(node) = stack.pop() {
            if node.is_none() || live[node.index()] {
                continue;
            }
            live[node.index()] = true;
            stack.extend(self.inputs(node).iter().copied());
            stack.push(self.control(node));
        }
        // Keep projections of live multi-result nodes that are themselves
        // used as control (they were reached through `control` above) — and
        // Phis of live regions whose values are used (reached as inputs).
        let mut mapping = vec![NodeId::NONE; count];
        let mut new_index = 0u32;
        for index in 0..count {
            if live[index] && !self.is_dead(NodeId(index as u32)) {
                mapping[index] = NodeId(new_index);
                new_index += 1;
            }
        }
        let remap = |node: NodeId| if node.is_none() { node } else { mapping[node.index()] };
        let mut records = Vec::with_capacity(new_index as usize);
        let mut control = Vec::with_capacity(new_index as usize);
        let mut types = Vec::with_capacity(new_index as usize);
        let mut extra = Vec::new();
        for (index, mapped) in mapping.iter().enumerate() {
            if mapped.is_none() {
                continue;
            }
            let node = NodeId(index as u32);
            let inputs: Vec<NodeId> = self.inputs(node).iter().map(|input| remap(*input)).collect();
            let mut record = self.records[index];
            if record.flags & FLAG_POOLED_INPUTS != 0 {
                record.in0 = NodeId(extra.len() as u32);
                record.in1 = NodeId(inputs.len() as u32);
                extra.extend(inputs);
            } else {
                record.in0 = inputs.first().copied().unwrap_or(NodeId::NONE);
                record.in1 = inputs.get(1).copied().unwrap_or(NodeId::NONE);
            }
            records.push(record);
            control.push(remap(self.control[index]));
            types.push(self.types[index]);
        }
        self.start = remap(self.start);
        self.forward = vec![NodeId::NONE; records.len()];
        self.records = records;
        self.control = control;
        self.types = types;
        self.extra = extra;
    }

    /// Projection `index` of `source`, if one exists.
    pub fn find_projection(&self, source: NodeId, index: u32) -> Option<NodeId> {
        self.ids().find(|node| {
            self.op(*node) == Opcode::Projection && self.payload(*node) == index && self.inputs(*node).first() == Some(&source)
        })
    }

    pub fn type_text(&self, ty: &ValueType) -> String {
        match ty {
            ValueType::Rust(ty) => self.oracle.describe(*ty),
            ValueType::Control => "Control".to_owned(),
            ValueType::Effect => "Effect".to_owned(),
            ValueType::Tuple(elements) => {
                let elements: Vec<String> = elements.iter().map(|element| self.type_text(element)).collect();
                format!("({})", elements.join(", "))
            }
        }
    }

    /// Stable text dump in the `docs/design/ir/nodes.md` format.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "graph {} ({:?}, {:?}) {{", self.symbol, self.level, self.state);
        for node in self.ids() {
            let op = self.op(node);
            let mut head = op.name().to_owned();
            match op {
                Opcode::Projection => head = format!("Projection[{}]", self.payload(node)),
                Opcode::Compare => {
                    head = format!("Compare.{}", ComparePredicate::from_payload(self.payload(node)).name())
                }
                Opcode::Const => head = format!("Const({})", self.constant_value(node) as i128),
                Opcode::Call => head = format!("Call({})", self.callees[self.payload(node) as usize].symbol),
                _ => {}
            }
            let _ = write!(out, "  {node} = {head}");
            let control = self.control(node);
            if !control.is_none() {
                let _ = write!(out, " ctl={control}");
            }
            let inputs = self.inputs(node);
            if !inputs.is_empty() {
                let inputs: Vec<String> = inputs.iter().map(|input| input.to_string()).collect();
                let _ = write!(out, " ({})", inputs.join(", "));
            }
            let _ = writeln!(out, " : {}", self.type_text(self.value_type(node)));
        }
        out.push('}');
        out
    }
}
