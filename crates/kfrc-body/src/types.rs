//! Types as the SoN sees them: interned `TyId`s with a structural shape.

use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TyId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntegerWidth {
    W8,
    W16,
    W32,
    W64,
    W128,
    /// `isize`/`usize`: pointer width (64 on the only PoC target).
    Pointer,
}

impl IntegerWidth {
    pub fn bits(self, pointer_bits: u8) -> u8 {
        match self {
            Self::W8 => 8,
            Self::W16 => 16,
            Self::W32 => 32,
            Self::W64 => 64,
            Self::W128 => 128,
            Self::Pointer => pointer_bits,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mutability {
    Not,
    Mut,
}

/// kfrc's own structural view of a type. Variants beyond the scalar ones are
/// representable now so that unsupported input is rejected by kind, not lost.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeShape {
    /// Definition-level type parameter; no layout is available until R→S.
    Param {
        index: u32,
        name: String,
    },
    Bool,
    Char,
    Int {
        width: IntegerWidth,
        signed: bool,
    },
    Float {
        bits: u8,
    },
    /// `()` and other zero-element tuples.
    Unit,
    Never,
    Tuple(Vec<TyId>),
    Array {
        element: TyId,
        length: u64,
    },
    Slice {
        element: TyId,
    },
    Str,
    Reference {
        mutability: Mutability,
        pointee: TyId,
    },
    RawPointer {
        mutability: Mutability,
        pointee: TyId,
    },
    /// IR storage address with a thin representation, including for DST places.
    /// Metadata belongs to the enclosing place, not to this address value.
    /// This is not a Rust raw pointer or reference type.
    PlaceAddress {
        pointee: TyId,
    },
    /// A struct, enum or union, by producer identity. Rustc producers include
    /// a stable fingerprint in this diagnostic label; names alone prove no
    /// built-in semantics. Logical fields are recorded separately.
    Adt {
        name: String,
    },
    /// A function item type: zero-sized, names one function.
    FunctionItem {
        name: String,
    },
    /// Anything the PoC does not model yet, by display name.
    Other {
        name: String,
    },
}

/// An append-only interner from shapes to `TyId`s.
#[derive(Clone, Debug, Default)]
pub struct TypeTable {
    shapes: Vec<TypeShape>,
    index: HashMap<TypeShape, TyId>,
}

impl TypeTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, shape: TypeShape) -> TyId {
        if let Some(id) = self.index.get(&shape) {
            return *id;
        }
        let id = TyId(self.shapes.len() as u32);
        self.shapes.push(shape.clone());
        self.index.insert(shape, id);
        id
    }

    pub fn shape(&self, ty: TyId) -> &TypeShape {
        &self.shapes[ty.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    pub fn display(&self, ty: TyId) -> TypeDisplay<'_> {
        TypeDisplay { table: self, ty }
    }
}

pub struct TypeDisplay<'a> {
    table: &'a TypeTable,
    ty: TyId,
}

impl fmt::Display for TypeDisplay<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.table.shape(self.ty) {
            TypeShape::Param { name, .. } => formatter.write_str(name),
            TypeShape::Bool => formatter.write_str("bool"),
            TypeShape::Char => formatter.write_str("char"),
            TypeShape::Int { width, signed } => {
                let prefix = if *signed { "i" } else { "u" };
                match width {
                    IntegerWidth::Pointer => write!(formatter, "{prefix}size"),
                    other => write!(formatter, "{prefix}{}", other.bits(64)),
                }
            }
            TypeShape::Float { bits } => write!(formatter, "f{bits}"),
            TypeShape::Unit => formatter.write_str("()"),
            TypeShape::Never => formatter.write_str("!"),
            TypeShape::Tuple(elements) => {
                formatter.write_str("(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{}", self.table.display(*element))?;
                }
                if elements.len() == 1 {
                    formatter.write_str(",")?;
                }
                formatter.write_str(")")
            }
            TypeShape::Array { element, length } => {
                write!(formatter, "[{}; {length}]", self.table.display(*element))
            }
            TypeShape::Slice { element } => write!(formatter, "[{}]", self.table.display(*element)),
            TypeShape::Str => formatter.write_str("str"),
            TypeShape::Reference {
                mutability,
                pointee,
            } => {
                let mutability = if *mutability == Mutability::Mut {
                    "mut "
                } else {
                    ""
                };
                write!(formatter, "&{mutability}{}", self.table.display(*pointee))
            }
            TypeShape::RawPointer {
                mutability,
                pointee,
            } => {
                let mutability = if *mutability == Mutability::Mut {
                    "mut"
                } else {
                    "const"
                };
                write!(formatter, "*{mutability} {}", self.table.display(*pointee))
            }
            TypeShape::PlaceAddress { pointee } => {
                write!(formatter, "addr({})", self.table.display(*pointee))
            }
            TypeShape::Adt { name }
            | TypeShape::FunctionItem { name }
            | TypeShape::Other { name } => formatter.write_str(name),
        }
    }
}
