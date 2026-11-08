//! The only way the middle and back end ask questions about types.
//!
//! The PoC implements it on top of rustc's `TyCtxt` (layout and ABI answers
//! are rustc's own, so mixed linking with rustc-compiled std is sound); the
//! product implements it on kfrc's front end.

use crate::types::{TyId, TypeShape, TypeTable};

/// How one argument or the return value crosses a call, named after
/// rustc's `PassMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PassMode {
    /// Zero-sized: not passed at all.
    Ignore,
    /// One integer register.
    Direct,
    /// Two registers (`&[T]`, `(i64, bool)`).
    Pair,
    /// Bit-cast into at most two eightbytes.
    Cast,
    /// Passed by pointer.
    Indirect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SizeAlign {
    pub size: u64,
    pub align: u64,
}

pub trait TypeOracle {
    fn shape(&self, ty: TyId) -> &TypeShape;

    fn size_align(&self, ty: TyId) -> Option<SizeAlign>;

    /// Human-readable type, for dumps and diagnostics.
    fn describe(&self, ty: TyId) -> String;
}

/// A self-contained oracle: shapes plus the layout answers recorded by the
/// producer. Bodies built by the converter carry one of these.
#[derive(Clone, Debug, Default)]
pub struct RecordedTypes {
    pub table: TypeTable,
    pub layouts: Vec<Option<SizeAlign>>,
}

impl RecordedTypes {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, shape: TypeShape, layout: Option<SizeAlign>) -> TyId {
        let ty = self.table.intern(shape);
        let index = ty.0 as usize;
        if self.layouts.len() <= index {
            self.layouts.resize(index + 1, None);
        }
        if self.layouts[index].is_none() {
            self.layouts[index] = layout;
        }
        ty
    }
}

impl TypeOracle for RecordedTypes {
    fn shape(&self, ty: TyId) -> &TypeShape {
        self.table.shape(ty)
    }

    fn size_align(&self, ty: TyId) -> Option<SizeAlign> {
        self.layouts.get(ty.0 as usize).copied().flatten()
    }

    fn describe(&self, ty: TyId) -> String {
        self.table.display(ty).to_string()
    }
}
