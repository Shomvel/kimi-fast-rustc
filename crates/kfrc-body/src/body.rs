//! Typed Body: MIR-shaped control flow over places (`docs/design/frontend/body.md`).
//!
//! All local reads and writes go through `Place`s; SSA is built once, by the
//! SoN builder. Statements of all blocks live in one pool and a block is a
//! range of it; projection lists and constants are pooled too.

use std::fmt::{self, Write as _};
use std::ops::Range;

use crate::oracle::{PassMode, RecordedTypes, TypeOracle};
use crate::types::{Mutability, TyId};

macro_rules! index_type {
    ($($(#[$meta:meta])* $name:ident;)*) => {$(
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);

        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    )*};
}

index_type! {
    /// `_0` is the return place, `_1..=arg_count` the arguments.
    LocalId;
    BlockId;
    ConstantId;
    /// Index into the projection-list pool; `ProjectionListId::EMPTY` is `[]`.
    ProjectionListId;
    FieldIndex;
    VariantIndex;
}

impl LocalId {
    pub const RETURN: Self = Self(0);
}

impl ProjectionListId {
    pub const EMPTY: Self = Self(0);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDecl {
    pub ty: TyId,
    pub mutability: Mutability,
    /// User variable name, for dumps only.
    pub name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    pub local: LocalId,
    pub projection: ProjectionListId,
}

impl Place {
    pub fn local(local: LocalId) -> Self {
        Self {
            local,
            projection: ProjectionListId::EMPTY,
        }
    }

    pub fn is_local(self) -> bool {
        self.projection == ProjectionListId::EMPTY
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ProjectionElem {
    Field(FieldIndex, TyId),
    Deref,
    Index(LocalId),
    ConstantIndex {
        offset: u64,
        min_length: u64,
        from_end: bool,
    },
    Subslice {
        from: u64,
        to: u64,
        from_end: bool,
        /// Exact projected type and layout recorded by the producer.
        result_ty: TyId,
    },
    Downcast(VariantIndex),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    Constant(ConstantId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constant {
    pub ty: TyId,
    pub kind: ConstantKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstantKind {
    /// A scalar's bits, zero-extended from its size.
    Scalar(u128),
    /// A relocation to a named data object or function.
    Address { symbol: String, addend: i64 },
    /// Any zero-sized value (unit, function items used as values).
    ZeroSized,
    /// A function item used as a call target.
    Function(Callee),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataRelocation {
    pub offset: u64,
    pub target: String,
    pub addend: i64,
}

/// Linker identity of a data definition, independent of writable/TLS storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DataBinding {
    /// Anonymous allocations belong to this compilation unit.
    #[default]
    Local,
    /// A named static with the visibility selected by its producer.
    Global { hidden: bool },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataObject {
    pub symbol: String,
    pub binding: DataBinding,
    pub bytes: Vec<u8>,
    pub align: u64,
    /// Writable target storage, including mutable statics and interior mutability.
    /// The default is read-only, even when relocations require load-time fixups.
    pub writable: bool,
    /// Initialization template for storage instantiated separately per thread.
    pub thread_local: bool,
    pub relocations: Vec<DataRelocation>,
}

/// What a call resolves to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Callee {
    /// A concrete function instance, by its linker symbol.
    Function { symbol: String, abi: FunctionAbi },
    /// A compiler intrinsic, by name (`wrapping_add`, `unreachable`, …).
    Intrinsic { name: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FunctionAbi {
    /// One entry per MIR argument, in order.
    pub args: Vec<PassMode>,
    pub ret: PassMode,
    pub c_variadic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Add,
    AddUnchecked,
    AddWithOverflow,
    Sub,
    SubUnchecked,
    SubWithOverflow,
    Mul,
    MulUnchecked,
    MulWithOverflow,
    Div,
    Rem,
    BitXor,
    BitAnd,
    BitOr,
    /// The shift amount is taken modulo the bit width.
    Shl,
    ShlUnchecked,
    Shr,
    ShrUnchecked,
    Eq,
    Lt,
    Le,
    Ne,
    Ge,
    Gt,
    /// Three-way comparison returning `Ordering`.
    Cmp,
    Offset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Not,
    Neg,
    PtrMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CastKind {
    IntToInt,
    IntToFloat,
    FloatToInt,
    FloatToFloat,
    PtrToPtr,
    Transmute,
    /// Any other cast kind, by name (unsized coercions, fn pointers, …).
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BorrowKind {
    Shared,
    Fake,
    Mut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AggregateKind {
    Tuple,
    Array(TyId),
    Adt {
        ty: TyId,
        variant: VariantIndex,
    },
    /// A union constructor supplies one operand for its selected field.
    Union {
        ty: TyId,
        field: FieldIndex,
    },
    Closure(TyId),
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rvalue {
    Use(Operand),
    /// Optimization barrier used by black_box; preserves value and effects.
    Opaque(Operand),
    /// Resolve the current thread's instance of a TLS symbol at this point.
    ThreadLocalRef {
        symbol: String,
    },
    Ref(BorrowKind, Place),
    RawPointer(Mutability, Place),
    BinaryOp(BinaryOp, Operand, Operand),
    UnaryOp(UnaryOp, Operand),
    Cast(CastKind, Operand, TyId),
    Aggregate(AggregateKind, Vec<Operand>),
    Repeat(Operand, u64),
    Discriminant(Place),
    Len(Place),
    /// A MIR rvalue the converter recognizes but the contract does not model
    /// yet; carried by name so the builder can reject it precisely.
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatementKind {
    Assign(Place, Rvalue),
    /// The predicate must be defined and true; failure is source UB.
    Assume(Operand),
    CopyNonOverlapping {
        src: Operand,
        dst: Operand,
        count: Operand,
        element_size: u64,
    },
    StorageLive(LocalId),
    StorageDead(LocalId),
    SetDiscriminant(Place, VariantIndex),
    Nop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnwindAction {
    Continue,
    Cleanup(BlockId),
    Terminate,
    Unreachable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssertKind {
    Overflow(BinaryOp),
    OverflowNeg,
    DivisionByZero,
    RemainderByZero,
    BoundsCheck,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchTargets {
    /// `(value, target)` pairs; values are distinct.
    pub cases: Vec<(u128, BlockId)>,
    pub otherwise: BlockId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminator {
    Goto(BlockId),
    SwitchInt {
        discriminant: Operand,
        targets: SwitchTargets,
    },
    Return,
    Unreachable,
    UnwindResume,
    UnwindTerminate,
    Call {
        callee: Operand,
        args: Vec<Operand>,
        destination: Place,
        target: Option<BlockId>,
        unwind: UnwindAction,
    },
    CallIndirect {
        function: Operand,
        abi: FunctionAbi,
        args: Vec<Operand>,
        destination: Place,
        target: Option<BlockId>,
        unwind: UnwindAction,
    },
    Assert {
        condition: Operand,
        expected: bool,
        message: AssertKind,
        target: BlockId,
        unwind: UnwindAction,
    },
    Drop {
        place: Place,
        target: BlockId,
        unwind: UnwindAction,
    },
}

impl Terminator {
    /// Normal (non-unwind) successors.
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Self::Goto(target) => vec![*target],
            Self::SwitchInt { targets, .. } => {
                let mut successors: Vec<BlockId> =
                    targets.cases.iter().map(|(_, target)| *target).collect();
                successors.push(targets.otherwise);
                successors
            }
            Self::Return | Self::Unreachable | Self::UnwindResume | Self::UnwindTerminate => {
                Vec::new()
            }
            Self::Call { target, .. } | Self::CallIndirect { target, .. } => {
                target.iter().copied().collect()
            }
            Self::Assert { target, .. } | Self::Drop { target, .. } => vec![*target],
        }
    }

    pub fn unwind(&self) -> Option<UnwindAction> {
        match self {
            Self::Call { unwind, .. }
            | Self::CallIndirect { unwind, .. }
            | Self::Assert { unwind, .. }
            | Self::Drop { unwind, .. } => Some(*unwind),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockData {
    pub statements: Range<u32>,
    pub terminator: Terminator,
    pub is_cleanup: bool,
}

#[derive(Clone, Debug)]
pub struct Body {
    /// Linker symbol of the function instance this body belongs to.
    pub symbol: String,
    pub locals: Vec<LocalDecl>,
    pub arg_count: usize,
    pub blocks: Vec<BlockData>,
    pub statements: Vec<StatementKind>,
    pub projections: Vec<Vec<ProjectionElem>>,
    pub constants: Vec<Constant>,
    pub data_objects: Vec<DataObject>,
    pub abi: FunctionAbi,
    pub types: RecordedTypes,
}

impl Body {
    pub fn local(&self, local: LocalId) -> &LocalDecl {
        &self.locals[local.index()]
    }

    pub fn block(&self, block: BlockId) -> &BlockData {
        &self.blocks[block.index()]
    }

    pub fn block_statements(&self, block: BlockId) -> &[StatementKind] {
        let range = &self.blocks[block.index()].statements;
        &self.statements[range.start as usize..range.end as usize]
    }

    pub fn projection(&self, id: ProjectionListId) -> &[ProjectionElem] {
        &self.projections[id.index()]
    }

    pub fn constant(&self, id: ConstantId) -> &Constant {
        &self.constants[id.index()]
    }

    pub fn return_ty(&self) -> TyId {
        self.locals[0].ty
    }

    /// Predecessor lists over normal edges.
    pub fn predecessors(&self) -> Vec<Vec<BlockId>> {
        let mut predecessors = vec![Vec::new(); self.blocks.len()];
        for (index, block) in self.blocks.iter().enumerate() {
            for successor in block.terminator.successors() {
                predecessors[successor.index()].push(BlockId(index as u32));
            }
        }
        predecessors
    }
}

/// Incremental construction with the pools kept consistent.
#[derive(Debug)]
pub struct BodyBuilder {
    body: Body,
    open_statements: Vec<StatementKind>,
}

impl BodyBuilder {
    pub fn new(symbol: impl Into<String>, abi: FunctionAbi, types: RecordedTypes) -> Self {
        Self {
            body: Body {
                symbol: symbol.into(),
                locals: Vec::new(),
                arg_count: 0,
                blocks: Vec::new(),
                statements: Vec::new(),
                projections: vec![Vec::new()],
                constants: Vec::new(),
                data_objects: Vec::new(),
                abi,
                types,
            },
            open_statements: Vec::new(),
        }
    }

    pub fn types(&mut self) -> &mut RecordedTypes {
        &mut self.body.types
    }

    pub fn add_local(&mut self, decl: LocalDecl) -> LocalId {
        self.body.locals.push(decl);
        LocalId(self.body.locals.len() as u32 - 1)
    }

    pub fn set_arg_count(&mut self, count: usize) {
        self.body.arg_count = count;
    }

    pub fn add_data_object(&mut self, object: DataObject) {
        if !self
            .body
            .data_objects
            .iter()
            .any(|existing| existing.symbol == object.symbol)
        {
            self.body.data_objects.push(object);
        }
    }

    pub fn add_constant(&mut self, constant: Constant) -> ConstantId {
        if let Some(index) = self
            .body
            .constants
            .iter()
            .position(|existing| *existing == constant)
        {
            return ConstantId(index as u32);
        }
        self.body.constants.push(constant);
        ConstantId(self.body.constants.len() as u32 - 1)
    }

    pub fn projection(&self, id: ProjectionListId) -> &[ProjectionElem] {
        &self.body.projections[id.index()]
    }

    pub fn constant(&self, id: ConstantId) -> Constant {
        self.body.constants[id.index()].clone()
    }

    pub fn add_projection(&mut self, elements: Vec<ProjectionElem>) -> ProjectionListId {
        if elements.is_empty() {
            return ProjectionListId::EMPTY;
        }
        if let Some(index) = self
            .body
            .projections
            .iter()
            .position(|existing| *existing == elements)
        {
            return ProjectionListId(index as u32);
        }
        self.body.projections.push(elements);
        ProjectionListId(self.body.projections.len() as u32 - 1)
    }

    pub fn push_statement(&mut self, statement: StatementKind) {
        self.open_statements.push(statement);
    }

    /// Closes the open block. Blocks must be finished in `BlockId` order.
    pub fn finish_block(&mut self, terminator: Terminator, is_cleanup: bool) -> BlockId {
        let start = self.body.statements.len() as u32;
        self.body.statements.append(&mut self.open_statements);
        let end = self.body.statements.len() as u32;
        self.body.blocks.push(BlockData {
            statements: start..end,
            terminator,
            is_cleanup,
        });
        BlockId(self.body.blocks.len() as u32 - 1)
    }

    pub fn finish(self) -> Body {
        self.body
    }
}

impl fmt::Display for Body {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        let _ = write!(out, "fn {}(", self.symbol);
        for index in 1..=self.arg_count {
            if index > 1 {
                out.push_str(", ");
            }
            let _ = write!(
                out,
                "_{index}: {}",
                self.types.describe(self.locals[index].ty)
            );
        }
        let _ = writeln!(out, ") -> {} {{", self.types.describe(self.return_ty()));
        for (index, local) in self.locals.iter().enumerate().skip(self.arg_count + 1) {
            let name = local
                .name
                .as_deref()
                .map(|name| format!(" // {name}"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "    let _{index}: {};{name}",
                self.types.describe(local.ty)
            );
        }
        for (index, block) in self.blocks.iter().enumerate() {
            let cleanup = if block.is_cleanup { " (cleanup)" } else { "" };
            let _ = writeln!(out, "  bb{index}{cleanup}: {{");
            for statement in self.block_statements(BlockId(index as u32)) {
                let _ = writeln!(out, "    {};", self.statement_text(statement));
            }
            let _ = writeln!(out, "    {};", self.terminator_text(&block.terminator));
            let _ = writeln!(out, "  }}");
        }
        out.push('}');
        formatter.write_str(&out)
    }
}

impl Body {
    pub fn place_text(&self, place: Place) -> String {
        let mut text = format!("_{}", place.local.0);
        for element in self.projection(place.projection) {
            text = match element {
                ProjectionElem::Field(field, _) => format!("{text}.{}", field.0),
                ProjectionElem::Deref => format!("(*{text})"),
                ProjectionElem::Index(local) => format!("{text}[_{}]", local.0),
                ProjectionElem::ConstantIndex { offset, .. } => format!("{text}[{offset}]"),
                ProjectionElem::Subslice { from, to, .. } => format!("{text}[{from}..{to}]"),
                ProjectionElem::Downcast(variant) => format!("({text} as variant#{})", variant.0),
            };
        }
        text
    }

    pub fn operand_text(&self, operand: &Operand) -> String {
        match operand {
            Operand::Copy(place) => self.place_text(*place),
            Operand::Move(place) => format!("move {}", self.place_text(*place)),
            Operand::Constant(id) => {
                let constant = self.constant(*id);
                match &constant.kind {
                    ConstantKind::Address { symbol, addend } => format!("&{symbol}{addend:+}"),
                    ConstantKind::Scalar(bits) => {
                        format!("const {bits}_{}", self.types.describe(constant.ty))
                    }
                    ConstantKind::ZeroSized => {
                        format!("const ZST::{}", self.types.describe(constant.ty))
                    }
                    ConstantKind::Function(Callee::Function { symbol, .. }) => {
                        format!("const fn {symbol}")
                    }
                    ConstantKind::Function(Callee::Intrinsic { name }) => {
                        format!("const intrinsic {name}")
                    }
                }
            }
        }
    }

    fn statement_text(&self, statement: &StatementKind) -> String {
        match statement {
            StatementKind::Assign(place, rvalue) => {
                let rvalue = match rvalue {
                    Rvalue::Use(operand) => self.operand_text(operand),
                    Rvalue::Opaque(operand) => format!("Opaque({})", self.operand_text(operand)),
                    Rvalue::Ref(kind, place) => format!("&{kind:?} {}", self.place_text(*place)),
                    Rvalue::RawPointer(mutability, place) => {
                        format!("&raw {mutability:?} {}", self.place_text(*place))
                    }
                    Rvalue::BinaryOp(op, lhs, rhs) => {
                        format!(
                            "{op:?}({}, {})",
                            self.operand_text(lhs),
                            self.operand_text(rhs)
                        )
                    }
                    Rvalue::UnaryOp(op, operand) => {
                        format!("{op:?}({})", self.operand_text(operand))
                    }
                    Rvalue::Cast(kind, operand, ty) => {
                        format!(
                            "{} as {} ({kind:?})",
                            self.operand_text(operand),
                            self.types.describe(*ty)
                        )
                    }
                    Rvalue::Repeat(operand, length) => {
                        format!("[{}; {length}]", self.operand_text(operand))
                    }
                    Rvalue::Aggregate(kind, operands) => {
                        let operands: Vec<String> = operands
                            .iter()
                            .map(|operand| self.operand_text(operand))
                            .collect();
                        format!("{kind:?}({})", operands.join(", "))
                    }
                    Rvalue::Discriminant(place) => {
                        format!("discriminant({})", self.place_text(*place))
                    }
                    Rvalue::Len(place) => format!("Len({})", self.place_text(*place)),
                    Rvalue::ThreadLocalRef { symbol } => format!("thread_local_address({symbol})"),
                    Rvalue::Unsupported(what) => format!("<unsupported {what}>"),
                };
                format!("{} = {rvalue}", self.place_text(*place))
            }
            StatementKind::Assume(predicate) => format!("assume({})", self.operand_text(predicate)),
            StatementKind::CopyNonOverlapping {
                src,
                dst,
                count,
                element_size,
            } => format!(
                "copy_nonoverlapping({}, {}, {}, element_size={element_size})",
                self.operand_text(src),
                self.operand_text(dst),
                self.operand_text(count)
            ),
            StatementKind::StorageLive(local) => format!("StorageLive(_{})", local.0),
            StatementKind::StorageDead(local) => format!("StorageDead(_{})", local.0),
            StatementKind::SetDiscriminant(place, variant) => {
                format!("discriminant({}) = {}", self.place_text(*place), variant.0)
            }
            StatementKind::Nop => "nop".to_owned(),
        }
    }

    fn terminator_text(&self, terminator: &Terminator) -> String {
        match terminator {
            Terminator::Goto(target) => format!("goto -> bb{}", target.0),
            Terminator::SwitchInt {
                discriminant,
                targets,
            } => {
                let cases: Vec<String> = targets
                    .cases
                    .iter()
                    .map(|(value, target)| format!("{value}: bb{}", target.0))
                    .collect();
                format!(
                    "switchInt({}) -> [{}, otherwise: bb{}]",
                    self.operand_text(discriminant),
                    cases.join(", "),
                    targets.otherwise.0
                )
            }
            Terminator::Return => "return".to_owned(),
            Terminator::Unreachable => "unreachable".to_owned(),
            Terminator::UnwindResume => "resume".to_owned(),
            Terminator::UnwindTerminate => "terminate".to_owned(),
            Terminator::Call {
                callee,
                args,
                destination,
                target,
                unwind,
            } => {
                let args: Vec<String> = args.iter().map(|arg| self.operand_text(arg)).collect();
                let target =
                    target.map_or("diverges".to_owned(), |target| format!("bb{}", target.0));
                format!(
                    "{} = {}({}) -> [return: {target}, unwind: {unwind:?}]",
                    self.place_text(*destination),
                    self.operand_text(callee),
                    args.join(", ")
                )
            }
            Terminator::CallIndirect {
                function,
                abi,
                args,
                destination,
                target,
                unwind,
            } => {
                let args: Vec<String> = args.iter().map(|arg| self.operand_text(arg)).collect();
                let target =
                    target.map_or("diverges".to_owned(), |target| format!("bb{}", target.0));
                format!(
                    "{} = indirect {}({}) [{abi:?}] -> [return: {target}, unwind: {unwind:?}]",
                    self.place_text(*destination),
                    self.operand_text(function),
                    args.join(", ")
                )
            }
            Terminator::Assert {
                condition,
                expected,
                message,
                target,
                unwind,
            } => format!(
                "assert({}{}, {message:?}) -> [success: bb{}, unwind: {unwind:?}]",
                if *expected { "" } else { "!" },
                self.operand_text(condition),
                target.0
            ),
            Terminator::Drop {
                place,
                target,
                unwind,
            } => {
                format!(
                    "drop({}) -> [return: bb{}, unwind: {unwind:?}]",
                    self.place_text(*place),
                    target.0
                )
            }
        }
    }
}
