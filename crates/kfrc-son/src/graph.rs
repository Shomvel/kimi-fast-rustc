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
    /// Semantic enum branch; payload indexes logical cases in `variant_switches`.
    /// Projections follow case order, then an otherwise edge.
    VariantSwitch,
    /// Compile-time variant evidence: enum value, proof control; payload is
    /// logical variant. Its control marks scope, never an execution event.
    VariantWitness,
    /// R/S UB requirement: effect,bool -> control,effect. No business value.
    Assume,
    /// Definition-level deferred question; payload indexes `holes`.
    Hole,
    /// Static dispatch deferred until specialization; payload indexes `trait_calls`.
    TraitCall,
    /// Inputs: the region, then one value per region predecessor.
    Phi,
    /// Inputs: effect, then the source return value in R/S. Only L Ignore
    /// returns omit the value after ABI transport selection.
    Return,
    /// Control reaches here only through undefined behaviour or divergence.
    Unreachable,
    /// `payload` indexes the callee table; inputs: effect, arguments.
    /// Produces `(Control, Effect, Value)`.
    Call,
    /// Indirect call: inputs effect,function,args; payload indexes call_abis.
    CallIndirect,
    /// Optimization barrier: effect,value -> control,effect,value.
    Opaque,
    /// `payload` indexes the constant pool.
    Const,
    /// A value read before any definition on some path (poison).
    Undefined,
    /// Rust aggregate value; inputs are logical fields in declaration order.
    Aggregate,
    /// Compact array repetition: one element operand; count is the result Array length.
    /// Payload is zero. A zero-length result does not observe its element.
    Repeat,
    /// Extract a logical field. Payload is the field index.
    Field,
    /// Replace a logical field: inputs old aggregate,value; payload field index.
    InsertField,
    /// Rust enum value; payload variant index, inputs that variant's fields.
    MakeVariant,
    /// Extract the logical variant index, independent of tag/niche encoding.
    VariantOf,
    /// Extract a variant field; payload indexes `variant_fields`.
    /// A different active variant produces poison, not an eager memory read.
    VariantField,
    /// Construct a wide pointer value: inputs data pointer,metadata.
    WidePtr,
    /// Source-level enum discriminant, obtained from the semantic type table.
    Discriminant,
    /// Logical local storage; payload is the source local identity, not a size.
    /// Result is a Rust thin pointer. Storage starts live and uninitialized;
    /// SlotLive/SlotDead refine its lifetime when explicitly present.
    LocalSlot,
    /// Logical field address; payload is a declaration-order field index.
    FieldAddr,
    /// Logical enum field address; payload indexes `variant_fields`.
    VariantFieldAddr,
    /// Pointer indexing: inputs base,index; stride is the result pointee type.
    /// A pointer step preserves the sized source pointee (including arrays);
    /// an array/slice/str place projection selects its immediate element type.
    /// Bounds are checked by source control and memory accesses, not this node.
    IndexAddr,
    /// Pointer retyping without changing its allocation identity or offset.
    PointerCast,
    /// Snapshot read: effect,address -> control,effect,Rust value. Aggregate
    /// fields retain their individual initialization state; only the active
    /// enum payload is interpreted. Payload indexes `memory_partitions`.
    LoadValue,
    /// Snapshot write: effect,address,value -> control,effect. Poison leaves
    /// remain uninitialized rather than poisoning unrelated initialized leaves.
    /// Payload indexes `memory_partitions`.
    StoreValue,
    /// Set an enum's active variant: effect,address,logical variant constant.
    /// Keeps previously written payload storage and its initialization state.
    /// Payload indexes `memory_partitions`.
    SetVariant,
    /// effect,address -> control,effect; activates storage and clears init state.
    /// Remains in L to retain lifetime facts until machine instruction selection.
    SlotLive,
    /// effect,address -> control,effect; ends storage lifetime and clears init state.
    /// Remains in L to retain lifetime facts until machine instruction selection.
    SlotDead,
    /// effect,source,destination,element count -> control,effect. Preserves raw
    /// initialized bytes and provenance; element type comes from source pointee.
    /// No operand encodes a byte size before layout lowering.
    CopyNonOverlapping,
    /// R/S semantic requirement: effect,value -> control,effect, no value result.
    /// Poison in active semantic leaves is UB; raw MaybeUninit bytes and padding
    /// remain legal. No memory access or complete type-validity check is implied.
    /// Publishing S→L erases this requirement before representation demand.
    RequireDefined,
    /// Fixed representation reinterpretation: effect,value -> control,effect,
    /// Rust target value. Invalid target decoding is immediate UB. Source and
    /// target sizes are checked at layout lowering, not guessed in the builder.
    TransmuteValue,
    /// A producer-validated constant representation, not a source scalar.
    /// Payload indexes `constants`; the result type determines storage width
    /// when interpreted/lowered. Widths above 128 bits are not representable.
    ConstRepr,
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
    /// Ordinary shifts mask their amount; AMOUNT_IN_RANGE instead makes an
    /// out-of-range amount poison. Arithmetic contracts live in payload.
    ShiftLeft,
    ShiftRightArithmetic,
    ShiftRightLogical,
    Negate,
    BitNot,
    /// `payload` is a `ComparePredicate`.
    Compare,
    /// Integer to integer conversion (truncate or extend by the types).
    IntCast,
    /// Address of a frame allocation; payload indexes `stack_slots`.
    StackSlot,
    /// Address of a linker symbol; payload indexes `symbols`.
    SymbolAddress,
    /// Current thread's symbol address: effect -> control,effect,thin pointer.
    /// Payload indexes symbols; unlike SymbolAddress this is fixed in order.
    ThreadLocalAddress,
    PtrOffset,
    /// Fixed memory read: (effect, address) -> (control, effect, value).
    Load,
    /// Fixed memory write: (effect, address, value) -> (control, effect).
    Store,
    /// Whether multiplying the two typed operands overflows their type.
    MulOverflow,
    /// Count bits; kind in low two payload bits, ZERO_POISON in bit 2. Result u32.
    BitCount,
    ByteSwap,
    BitReverse,
    /// hi,lo,amount; direction in bit 0, AMOUNT_IN_RANGE in bit 1.
    FunnelShift,
    SaturatingAdd,
    SaturatingSub,
}

impl Opcode {
    /// Value operations that must be eliminated by layout lowering.
    pub fn is_semantic_value(self) -> bool {
        matches!(
            self,
            Self::Aggregate
                | Self::Repeat
                | Self::Field
                | Self::InsertField
                | Self::MakeVariant
                | Self::VariantOf
                | Self::VariantField
                | Self::WidePtr
                | Self::Discriminant
                | Self::ConstRepr
        )
    }

    /// Storage operations requiring layout lowering. Lifetime markers survive
    /// the boundary and deliberately are not part of this set.
    pub fn is_semantic_memory(self) -> bool {
        matches!(
            self,
            Self::LocalSlot
                | Self::FieldAddr
                | Self::VariantFieldAddr
                | Self::IndexAddr
                | Self::PointerCast
                | Self::LoadValue
                | Self::StoreValue
                | Self::SetVariant
                | Self::CopyNonOverlapping
                | Self::TransmuteValue
        )
    }

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
                | Self::VariantSwitch
                | Self::Assume
                | Self::TraitCall
                | Self::Return
                | Self::Unreachable
                | Self::Call
                | Self::CallIndirect
                | Self::Opaque
                | Self::RequireDefined
                | Self::LoadValue
                | Self::StoreValue
                | Self::SetVariant
                | Self::SlotLive
                | Self::SlotDead
                | Self::CopyNonOverlapping
                | Self::TransmuteValue
                | Self::ThreadLocalAddress
                | Self::Load
                | Self::Store
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
            Self::VariantSwitch => "VariantSwitch",
            Self::VariantWitness => "VariantWitness",
            Self::Assume => "Assume",
            Self::Hole => "Hole",
            Self::TraitCall => "TraitCall",
            Self::Phi => "Phi",
            Self::Return => "Return",
            Self::Unreachable => "Unreachable",
            Self::Call => "Call",
            Self::CallIndirect => "CallIndirect",
            Self::Opaque => "Opaque",
            Self::Const => "Const",
            Self::Undefined => "Undefined",
            Self::Aggregate => "Aggregate",
            Self::Repeat => "Repeat",
            Self::Field => "Field",
            Self::InsertField => "InsertField",
            Self::MakeVariant => "MakeVariant",
            Self::VariantOf => "VariantOf",
            Self::VariantField => "VariantField",
            Self::WidePtr => "WidePtr",
            Self::Discriminant => "Discriminant",
            Self::LocalSlot => "LocalSlot",
            Self::FieldAddr => "FieldAddr",
            Self::VariantFieldAddr => "VariantFieldAddr",
            Self::IndexAddr => "IndexAddr",
            Self::PointerCast => "PointerCast",
            Self::LoadValue => "LoadValue",
            Self::StoreValue => "StoreValue",
            Self::SetVariant => "SetVariant",
            Self::SlotLive => "SlotLive",
            Self::SlotDead => "SlotDead",
            Self::CopyNonOverlapping => "CopyNonOverlapping",
            Self::RequireDefined => "RequireDefined",
            Self::TransmuteValue => "TransmuteValue",
            Self::ConstRepr => "ConstRepr",
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
            Self::StackSlot => "StackSlot",
            Self::SymbolAddress => "SymbolAddress",
            Self::ThreadLocalAddress => "ThreadLocalAddress",
            Self::PtrOffset => "PtrOffset",
            Self::Load => "Load",
            Self::Store => "Store",
            Self::MulOverflow => "MulOverflow",
            Self::BitCount => "BitCount",
            Self::ByteSwap => "ByteSwap",
            Self::BitReverse => "BitReverse",
            Self::FunnelShift => "FunnelShift",
            Self::SaturatingAdd => "SaturatingAdd",
            Self::SaturatingSub => "SaturatingSub",
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
    /// A source-language value, never an implicit aggregate address in R/S.
    Rust(TyId),
    /// L-only aggregate snapshot stored at an address for the physical ABI.
    /// This is not a source-language reference to the value.
    LayoutValue(TyId),
    /// Compile-time evidence, never a Rust or machine value.
    Witness,
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

/// A proven memory region, never inferred just from reference mutability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemoryPartition {
    Unknown,
    /// A LocalSlot identity whose address does not escape this function.
    Local {
        local: u32,
    },
    /// An immutable DataObject, referenced by its entry in `symbols`.
    Const {
        symbol: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphState {
    Building,
    Sealed,
    Verified,
}

/// Rust borrow permission of one direct parameter (RS5.1, RF20/RF21).
///
/// The scope is one execution of the instance body that owns it (the
/// parameter protector). The producer follows the rustc-O contract (D1): the
/// permission is implied by the `noalias`/`readonly`/`dereferenceable`
/// attributes rustc 1.94.1 emits for the same instance when optimizing.
/// `dereferenceable` is the pointee size for sized pointees and `None` for
/// unsized pointees. Layout lowering erases every permission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ArgPermission {
    #[default]
    None,
    /// `&T` with `T: Freeze`: nobody writes the pointee during the call.
    Frozen { dereferenceable: Option<u64> },
    /// `&mut T` with `T: Unpin` under `-Zmutable-noalias`: accesses through
    /// other pointers do not alias the pointee during the call.
    Unique { dereferenceable: Option<u64> },
}

impl ArgPermission {
    pub fn is_frozen(self) -> bool {
        matches!(self, Self::Frozen { .. })
    }

    fn text(self) -> String {
        let bytes = |size: Option<u64>| size.map_or("unsized".to_owned(), |size| size.to_string());
        match self {
            Self::None => "None".to_owned(),
            Self::Frozen { dereferenceable } => format!("Frozen({})", bytes(dereferenceable)),
            Self::Unique { dereferenceable } => format!("Unique({})", bytes(dereferenceable)),
        }
    }
}

/// A direct call target: a function instance by linker symbol.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallTarget {
    pub symbol: String,
    pub abi: kfrc_body::FunctionAbi,
}

/// Source signature. R never consults physical passing modes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RustSignature {
    pub args: Vec<TyId>,
    pub ret: TyId,
}

/// Questions are deduplicated in deterministic first-use order. A hole is not
/// storage; no query is answered until a reachable consumer demands it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Hole {
    SizeAlign(TyId),
    NeedsDrop(TyId),
    Resolve { item: String, args: Vec<TyId> },
    VariantEncoding(TyId),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TraitCall {
    /// Index of a Resolve hole, never a runtime virtual dispatch.
    pub resolve: u32,
    pub signature: RustSignature,
}

#[derive(Clone, Debug)]
pub struct FunctionGraph {
    pub symbol: String,
    pub abi: kfrc_body::FunctionAbi,
    /// Present exactly for definition-level R graphs.
    pub rust_signature: Option<RustSignature>,
    pub holes: Vec<Hole>,
    pub trait_calls: Vec<TraitCall>,
    pub variant_switches: Vec<Vec<u32>>,
    records: Vec<NodeRecord>,
    control: Vec<NodeId>,
    types: Vec<ValueTypeId>,
    extra: Vec<NodeId>,
    forward: Vec<NodeId>,
    value_types: Vec<ValueType>,
    pub constants: Vec<u128>,
    pub callees: Vec<CallTarget>,
    /// Permissions of S parameters in `Start` order. Empty means all `None`;
    /// R graphs and Layout graphs keep it empty. Kept apart from `abi` so
    /// function-pointer ABI equality is unchanged.
    pub arg_permissions: Vec<ArgPermission>,
    /// Parameter permissions of direct call targets, by callee symbol and call
    /// argument index. A symbol names one instance, so this table survives
    /// callee interning, splicing and lowering-added callees unchanged.
    pub callee_permissions: std::collections::BTreeMap<String, Vec<ArgPermission>>,
    pub call_abis: Vec<kfrc_body::FunctionAbi>,
    pub stack_slots: Vec<kfrc_body::SizeAlign>,
    /// L-level physical slot index -> source LocalSlot identity. ABI temporary
    /// slots are absent; lowering preserves this evidence for Local partitions.
    pub stack_slot_locals: std::collections::HashMap<u32, u32>,
    pub symbols: Vec<(String, i64)>,
    pub data_objects: Vec<kfrc_body::DataObject>,
    /// Full-width logical (variant,field) indices for VariantField nodes.
    pub variant_fields: Vec<(u32, u32)>,
    /// Entry zero is always Unknown. Memory operation payloads index this table.
    /// Local and Const entries require corresponding escape/immutability proof.
    pub memory_partitions: Vec<MemoryPartition>,
    pub start: NodeId,
    pub state: GraphState,
    pub level: GraphLevel,
    /// Answers about Rust types (P0: recorded by the Body producer).
    pub oracle: RecordedTypes,
}

impl FunctionGraph {
    pub fn new(
        symbol: impl Into<String>,
        abi: kfrc_body::FunctionAbi,
        oracle: RecordedTypes,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            abi,
            rust_signature: None,
            holes: Vec::new(),
            trait_calls: Vec::new(),
            variant_switches: Vec::new(),
            records: Vec::new(),
            control: Vec::new(),
            types: Vec::new(),
            extra: Vec::new(),
            forward: Vec::new(),
            value_types: Vec::new(),
            constants: Vec::new(),
            callees: Vec::new(),
            arg_permissions: Vec::new(),
            callee_permissions: std::collections::BTreeMap::new(),
            call_abis: Vec::new(),
            stack_slots: Vec::new(),
            stack_slot_locals: std::collections::HashMap::new(),
            symbols: Vec::new(),
            data_objects: Vec::new(),
            variant_fields: Vec::new(),
            memory_partitions: vec![MemoryPartition::Unknown],
            start: NodeId::NONE,
            state: GraphState::Building,
            level: GraphLevel::Specialized,
            oracle,
        }
    }

    /// Construct R without querying or inventing physical argument modes.
    pub fn new_rust(
        symbol: impl Into<String>,
        signature: RustSignature,
        oracle: RecordedTypes,
    ) -> Self {
        let mut graph = Self::new(
            symbol,
            kfrc_body::FunctionAbi {
                args: Vec::new(),
                ret: kfrc_body::PassMode::Ignore,
                c_variadic: false,
            },
            oracle,
        );
        graph.level = GraphLevel::Rust;
        graph.rust_signature = Some(signature);
        graph
    }

    pub fn add_hole(&mut self, hole: Hole) -> u32 {
        if let Some(index) = self.holes.iter().position(|existing| existing == &hole) {
            return index as u32;
        }
        self.invalidate();
        self.holes.push(hole);
        self.holes.len() as u32 - 1
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Any semantic graph edit revokes a previous verification certificate.
    pub fn invalidate(&mut self) {
        if self.state == GraphState::Verified {
            self.state = GraphState::Sealed;
        }
    }

    pub fn intern_type(&mut self, ty: ValueType) -> ValueTypeId {
        if let Some(index) = self.value_types.iter().position(|existing| *existing == ty) {
            return ValueTypeId(index as u32);
        }
        self.invalidate();
        self.value_types.push(ty);
        ValueTypeId(self.value_types.len() as u32 - 1)
    }

    pub fn add_node(
        &mut self,
        op: Opcode,
        control: NodeId,
        inputs: &[NodeId],
        payload: u32,
        ty: ValueType,
    ) -> NodeId {
        self.invalidate();
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
            (
                FLAG_POOLED_INPUTS,
                NodeId(start),
                NodeId(inputs.len() as u32),
            )
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
    pub fn add_growable_node(
        &mut self,
        op: Opcode,
        control: NodeId,
        count: usize,
        payload: u32,
        ty: ValueType,
    ) -> NodeId {
        self.invalidate();
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

    pub fn set_value_type(&mut self, node: NodeId, ty: ValueType) {
        self.invalidate();
        let ty = self.intern_type(ty);
        self.types[node.index()] = ty;
    }

    pub fn set_opcode(&mut self, node: NodeId, op: Opcode) {
        self.invalidate();
        self.records[node.index()].op = op;
    }

    pub fn set_payload(&mut self, node: NodeId, payload: u32) {
        self.invalidate();
        self.records[node.index()].payload = payload;
    }

    pub fn set_opcode_payload(&mut self, node: NodeId, op: Opcode, payload: u32) {
        self.invalidate();
        self.records[node.index()].op = op;
        self.records[node.index()].payload = payload;
    }

    pub fn intern_variant_field(&mut self, variant: u32, field: u32) -> u32 {
        let key = (variant, field);
        if let Some(index) = self.variant_fields.iter().position(|entry| *entry == key) {
            return index as u32;
        }
        self.invalidate();
        self.variant_fields.push(key);
        self.variant_fields.len() as u32 - 1
    }

    pub fn intern_memory_partition(&mut self, partition: MemoryPartition) -> u32 {
        if let Some(index) = self
            .memory_partitions
            .iter()
            .position(|entry| *entry == partition)
        {
            return index as u32;
        }
        self.invalidate();
        self.memory_partitions.push(partition);
        self.memory_partitions.len() as u32 - 1
    }

    pub fn rust_type(&self, node: NodeId) -> Option<TyId> {
        match self.value_type(node) {
            ValueType::Rust(ty) | ValueType::LayoutValue(ty) => Some(*ty),
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
        self.invalidate();
        let record = self.records[node.index()];
        if record.flags & FLAG_POOLED_INPUTS != 0 {
            assert!(
                index < record.in1.index(),
                "pooled input index out of range"
            );
            self.extra[record.in0.index() + index] = value;
        } else {
            match index {
                0 => self.records[node.index()].in0 = value,
                1 => self.records[node.index()].in1 = value,
                _ => panic!("inline nodes have at most two inputs"),
            }
        }
    }

    /// Replaces an entire operand list, including Region/Phi arity changes.
    /// Structural editors must update related nodes together before verification.
    pub fn set_inputs(&mut self, node: NodeId, inputs: &[NodeId]) {
        self.invalidate();
        let start = self.extra.len();
        self.extra.extend_from_slice(inputs);
        let record = &mut self.records[node.index()];
        record.flags |= FLAG_POOLED_INPUTS;
        record.in0 = NodeId(start as u32);
        record.in1 = NodeId(inputs.len() as u32);
    }

    pub fn set_control(&mut self, node: NodeId, control: NodeId) {
        self.invalidate();
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
        self.add_node(
            Opcode::Const,
            NodeId::NONE,
            &[],
            index as u32,
            ValueType::Rust(ty),
        )
    }

    pub fn add_repr_constant(&mut self, bits: u128, ty: TyId) -> NodeId {
        let node = self.add_constant(bits, ty);
        self.set_opcode(node, Opcode::ConstRepr);
        node
    }

    /// Permission of S parameter `parameter` (0-based, `Start` projection 2+).
    pub fn arg_permission(&self, parameter: usize) -> ArgPermission {
        self.arg_permissions
            .get(parameter)
            .copied()
            .unwrap_or_default()
    }

    /// Permission of argument `argument` (0-based, after the effect input) of
    /// a direct call through callee table entry `callee`.
    pub fn callee_permission(&self, callee: u32, argument: usize) -> ArgPermission {
        self.callees
            .get(callee as usize)
            .and_then(|target| self.callee_permissions.get(&target.symbol))
            .and_then(|permissions| permissions.get(argument))
            .copied()
            .unwrap_or_default()
    }

    pub fn add_callee(&mut self, target: CallTarget) -> u32 {
        if let Some(index) = self.callees.iter().position(|existing| *existing == target) {
            return index as u32;
        }
        self.invalidate();
        self.callees.push(target);
        self.callees.len() as u32 - 1
    }

    /// `node` is replaced by `replacement` everywhere (applied by `resolve`
    /// and `compact`).
    pub fn replace(&mut self, node: NodeId, replacement: NodeId) {
        let replacement = self.resolve(replacement);
        if node != replacement {
            self.invalidate();
            self.forward[node.index()] = replacement;
            self.records[node.index()].flags |= FLAG_DEAD;
        }
    }

    /// Retires a node after its users have been rewritten or retired. This does
    /// not erase or renumber its storage; editors own the surrounding invariant.
    pub fn remove_node(&mut self, node: NodeId) {
        self.invalidate();
        self.records[node.index()].flags |= FLAG_DEAD;
    }

    /// Applies forwarding without renumbering NodeIds. Analyses still need to be
    /// rebuilt after mutations; only pipeline finalization calls `compact`.
    pub fn resolve_edges(&mut self) {
        let nodes: Vec<_> = self.ids().collect();
        for node in nodes {
            let inputs: Vec<_> = self
                .inputs(node)
                .iter()
                .map(|input| self.resolve(*input))
                .collect();
            for (slot, input) in inputs.into_iter().enumerate() {
                if self.input(node, slot) != input {
                    self.set_input(node, slot, input);
                }
            }
            let control = self.resolve(self.control(node));
            if self.control(node) != control {
                self.set_control(node, control);
            }
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
        (0..self.records.len() as u32)
            .map(NodeId)
            .filter(|node| !self.is_dead(*node))
    }

    /// Rewrites every input through the forwarding table, removes nodes not
    /// reachable from the control skeleton, and renumbers in creation order.
    pub fn compact(&mut self) {
        self.invalidate();
        self.resolve_edges();
        let count = self.records.len();
        // All fixed operations are observable roots, including unused trapping
        // arithmetic. CFG transforms retire unreachable roots before compaction.
        let mut live = vec![false; count];
        let mut stack: Vec<NodeId> = self
            .ids()
            .filter(|node| self.op(*node).is_fixed())
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
        let remap = |node: NodeId| {
            if node.is_none() {
                node
            } else {
                mapping[node.index()]
            }
        };
        let mut records = Vec::with_capacity(new_index as usize);
        let mut control = Vec::with_capacity(new_index as usize);
        let mut types = Vec::with_capacity(new_index as usize);
        let mut extra = Vec::new();
        for (index, mapped) in mapping.iter().enumerate() {
            if mapped.is_none() {
                continue;
            }
            let node = NodeId(index as u32);
            let inputs: Vec<NodeId> = self
                .inputs(node)
                .iter()
                .map(|input| remap(*input))
                .collect();
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
            self.op(*node) == Opcode::Projection
                && self.payload(*node) == index
                && self.inputs(*node).first() == Some(&source)
        })
    }

    pub fn type_text(&self, ty: &ValueType) -> String {
        match ty {
            ValueType::Rust(ty) => self.oracle.describe(*ty),
            ValueType::LayoutValue(ty) => format!("LayoutValue<{}>", self.oracle.describe(*ty)),
            ValueType::Witness => "Witness".to_owned(),
            ValueType::Control => "Control".to_owned(),
            ValueType::Effect => "Effect".to_owned(),
            ValueType::Tuple(elements) => {
                let elements: Vec<String> = elements
                    .iter()
                    .map(|element| self.type_text(element))
                    .collect();
                format!("({})", elements.join(", "))
            }
        }
    }

    /// Stable text dump in the `docs/design/ir/nodes.md` format.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "graph {} ({:?}, {:?}) {{",
            self.symbol, self.level, self.state
        );
        if self
            .arg_permissions
            .iter()
            .any(|permission| *permission != ArgPermission::None)
        {
            let permissions: Vec<String> = self
                .arg_permissions
                .iter()
                .enumerate()
                .map(|(parameter, permission)| format!("p{parameter}:{}", permission.text()))
                .collect();
            let _ = writeln!(out, "  permissions {}", permissions.join(" "));
        }
        for node in self.ids() {
            let op = self.op(node);
            let mut head = op.name().to_owned();
            match op {
                Opcode::BitCount | Opcode::FunnelShift => {
                    head = format!("{}[{}]", op.name(), self.payload(node))
                }
                Opcode::Projection => head = format!("Projection[{}]", self.payload(node)),
                Opcode::Field
                | Opcode::InsertField
                | Opcode::MakeVariant
                | Opcode::FieldAddr
                | Opcode::LocalSlot => {
                    head = format!("{}[{}]", op.name(), self.payload(node));
                }
                Opcode::VariantField | Opcode::VariantFieldAddr => {
                    head = match self.variant_fields.get(self.payload(node) as usize) {
                        Some((variant, field)) => format!("{}[{variant},{field}]", op.name()),
                        None => format!("{}[invalid:{}]", op.name(), self.payload(node)),
                    };
                }
                Opcode::LoadValue
                | Opcode::StoreValue
                | Opcode::SetVariant
                | Opcode::SlotLive
                | Opcode::SlotDead
                | Opcode::CopyNonOverlapping => {
                    head = format!("{}[partition:{}]", op.name(), self.payload(node));
                }
                Opcode::Compare => {
                    head = format!(
                        "Compare.{}",
                        ComparePredicate::from_payload(self.payload(node)).name()
                    )
                }
                Opcode::Const => head = format!("Const({})", self.constant_value(node) as i128),
                Opcode::ConstRepr => head = format!("ConstRepr({:#x})", self.constant_value(node)),
                Opcode::Call => {
                    head = format!("Call({})", self.callees[self.payload(node) as usize].symbol)
                }
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
