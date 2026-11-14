//! The only way the middle and back end ask questions about types.
//!
//! The PoC implements it on top of rustc's `TyCtxt` (layout and ABI answers
//! are rustc's own, so mixed linking with rustc-compiled std is sound); the
//! product implements it on kfrc's front end.

use crate::fx_hash::{HashMap, HashSet};
use crate::types::{TyId, TypeShape, TypeTable};
use crate::{ScalarValidRange, ScalarValidity};

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
    /// One SIMD register selected by the producer's actual function ABI.
    Vector { bytes: u8 },
    /// Bit-cast into at most two eightbytes.
    Cast(CastAbi),
    /// Passed by pointer.
    Indirect,
    /// C ABI by-value memory argument; not implemented by this PoC.
    IndirectOnStack,
}

/// The exact integer register representation selected by the producer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CastAbi {
    pub size: u64,
    pub lanes: [Option<CastLane>; 2],
    pub consecutive: bool,
    pub pad_i32: bool,
    pub extension: CastExtension,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CastLane {
    pub offset: u64,
    /// Physical register lane width in bytes, including partial integer types.
    pub bytes: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CastExtension {
    None,
    Zero,
    Sign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SizeAlign {
    pub size: u64,
    pub align: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldLayout {
    pub ty: TyId,
    pub offset: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AggregateLayout {
    pub fields: Vec<FieldLayout>,
    pub variants: Vec<Vec<FieldLayout>>,
}

/// Logical aggregate members, independent of offsets, padding and ABI lanes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SemanticAggregate {
    pub fields: Vec<TyId>,
    pub variants: Vec<Vec<TyId>>,
}

/// A semantic-only view. Consumers of this view cannot query size, alignment,
/// offsets, tag encodings or calling conventions.
#[derive(Clone, Copy)]
pub struct SemanticTypes<'a> {
    table: &'a TypeTable,
    aggregates: &'a HashMap<TyId, SemanticAggregate>,
    discriminants: &'a HashMap<TyId, Vec<u128>>,
    inhabited_variants: &'a HashMap<TyId, Vec<u32>>,
    unions: &'a HashSet<TyId>,
    maybe_uninit: &'a HashMap<TyId, TyId>,
    unsized_types: &'a HashSet<TyId>,
    thin_unsized: &'a HashSet<TyId>,
    static_slice_tails: &'a HashMap<TyId, u32>,
    ordering: &'a HashSet<TyId>,
    vectors: &'a HashMap<TyId, VectorLayout>,
}

impl SemanticTypes<'_> {
    pub fn shape(&self, ty: TyId) -> &TypeShape {
        self.table.shape(ty)
    }

    /// Producer-authenticated Rust MaybeUninit identity, never a name heuristic.
    pub fn maybe_uninit_inner(&self, ty: TyId) -> Option<TyId> {
        self.maybe_uninit.get(&ty).copied()
    }

    /// Sizedness is a source fact, not absence of a physical layout in R.
    pub fn is_unsized(&self, ty: TyId) -> bool {
        matches!(self.shape(ty), TypeShape::Slice { .. } | TypeShape::Str)
            || self.unsized_types.contains(&ty)
    }

    /// Extern types (and their struct tails) are unsized with unit metadata.
    pub fn is_thin_unsized(&self, ty: TyId) -> bool {
        self.thin_unsized.contains(&ty)
    }

    pub fn requires_pointer_metadata(&self, ty: TyId) -> bool {
        self.is_unsized(ty) && !self.is_thin_unsized(ty)
    }

    /// A producer-verified struct tail with static offset and slice metadata.
    /// Dynamic-alignment tails such as dyn Trait do not carry this fact.
    pub fn static_slice_tail(&self, ty: TyId) -> Option<u32> {
        self.static_slice_tails.get(&ty).copied()
    }

    pub fn is_ordering(&self, ty: TyId) -> bool {
        self.ordering.contains(&ty)
    }

    /// Logical owner of an internal storage address or a real thin Rust pointer.
    /// A Rust pointer to a DST is never reclassified as thin due to missing
    /// wide-pointer fields; that is incomplete producer metadata instead.
    pub fn thin_pointee(&self, ty: TyId) -> Option<TyId> {
        match self.shape(ty) {
            TypeShape::PlaceAddress { pointee } => Some(*pointee),
            TypeShape::Reference { pointee, .. } | TypeShape::RawPointer { pointee, .. }
                if !self.is_product(ty) && !self.requires_pointer_metadata(*pointee) =>
            {
                Some(*pointee)
            }
            _ => None,
        }
    }

    /// Number of logical fields without materializing an array-sized vector.
    pub fn field_count(&self, ty: TyId, variant: Option<u32>) -> Option<u64> {
        if let Some(variant) = variant {
            if !self.is_enum(ty) {
                return None;
            }
            return self
                .aggregates
                .get(&ty)?
                .variants
                .get(variant as usize)
                .map(|fields| fields.len() as u64);
        }
        if self.is_enum(ty) {
            return None;
        }
        match self.shape(ty) {
            TypeShape::Unit => Some(0),
            TypeShape::Tuple(fields) => Some(fields.len() as u64),
            TypeShape::Array { length, .. } => Some(*length),
            _ => self
                .aggregates
                .get(&ty)
                .map(|aggregate| aggregate.fields.len() as u64),
        }
    }

    /// A logical product, including arrays and wide pointer components.
    /// Enum variants require an explicit variant and are not products here.
    pub fn is_product(&self, ty: TyId) -> bool {
        self.maybe_uninit_inner(ty).is_none() && self.field_count(ty, None).is_some()
    }

    pub fn field_types(&self, ty: TyId, variant: Option<u32>) -> Option<Vec<TyId>> {
        if let Some(variant) = variant {
            if !self.is_enum(ty) {
                return None;
            }
            return self
                .aggregates
                .get(&ty)?
                .variants
                .get(variant as usize)
                .cloned();
        }
        if self.is_enum(ty) {
            return None;
        }
        match self.shape(ty) {
            TypeShape::Unit => Some(Vec::new()),
            TypeShape::Tuple(fields) => Some(fields.clone()),
            TypeShape::Array { element, length } => {
                let length = usize::try_from(*length).ok()?;
                Some(vec![*element; length])
            }
            _ => self
                .aggregates
                .get(&ty)
                .map(|aggregate| aggregate.fields.clone()),
        }
    }

    pub fn field_type(&self, ty: TyId, variant: Option<u32>, index: u32) -> Option<TyId> {
        if let Some(variant) = variant {
            if !self.is_enum(ty) {
                return None;
            }
            return self
                .aggregates
                .get(&ty)?
                .variants
                .get(variant as usize)?
                .get(index as usize)
                .copied();
        }
        if self.is_enum(ty) {
            return None;
        }
        match self.shape(ty) {
            TypeShape::Tuple(fields) => fields.get(index as usize).copied(),
            TypeShape::Array { element, length } => {
                (u64::from(index) < *length).then_some(*element)
            }
            _ => self
                .aggregates
                .get(&ty)?
                .fields
                .get(index as usize)
                .copied(),
        }
    }

    pub fn is_enum(&self, ty: TyId) -> bool {
        self.discriminants.contains_key(&ty)
    }

    pub fn variant_count(&self, ty: TyId) -> Option<usize> {
        if !self.is_enum(ty) {
            return None;
        }
        self.aggregates
            .get(&ty)
            .map(|aggregate| aggregate.variants.len())
    }

    /// Variants the producer proved can be inhabited. `None` means unknown,
    /// as for a generic enum whose field types have not been instantiated.
    /// This is a semantic fact, independent of tag encoding and storage size.
    pub fn inhabited_variants(&self, ty: TyId) -> Option<&[u32]> {
        self.inhabited_variants.get(&ty).map(Vec::as_slice)
    }

    /// Source-level discriminant bits, never the physical tag or niche value.
    /// Producers must register these even for niche and single-variant enums.
    pub fn discriminant(&self, ty: TyId, variant: u32) -> Option<u128> {
        self.discriminants.get(&ty)?.get(variant as usize).copied()
    }

    pub fn is_union(&self, ty: TyId) -> bool {
        self.unions.contains(&ty)
    }

    pub fn is_vector(&self, ty: TyId) -> bool {
        self.vectors.contains_key(&ty)
    }
}

/// Physical SIMD lane representation, separate from nominal ADT fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VectorLayout {
    pub element: TyId,
    pub lanes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagEncoding {
    Empty,
    Single {
        variant: u32,
        discriminant: u128,
    },
    Direct {
        discriminants: Vec<u128>,
    },
    Niche {
        untagged_variant: u32,
        niche_variants: std::ops::RangeInclusive<u32>,
        niche_start: u128,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumLayout {
    pub tag: Option<FieldLayout>,
    pub encoding: TagEncoding,
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
    pub aggregates: HashMap<TyId, AggregateLayout>,
    /// ScalarPair lanes in physical ABI order, with their memory offsets.
    pub abi_pairs: HashMap<TyId, [FieldLayout; 2]>,
    pub enums: HashMap<TyId, EnumLayout>,
    /// Union fields overlap; copies preserve bytes without interpreting them.
    pub unions: HashSet<TyId>,
    pub vectors: HashMap<TyId, VectorLayout>,
    maybe_uninit: HashMap<TyId, TyId>,
    unsized_types: HashSet<TyId>,
    thin_unsized: HashSet<TyId>,
    static_slice_tails: HashMap<TyId, u32>,
    ordering: HashSet<TyId>,
    semantic_aggregates: HashMap<TyId, SemanticAggregate>,
    semantic_discriminants: HashMap<TyId, Vec<u128>>,
    semantic_inhabited_variants: HashMap<TyId, Vec<u32>>,
    scalar_validity: HashMap<TyId, ScalarValidity>,
}

impl RecordedTypes {
    pub fn new() -> Self {
        Self::default()
    }

    /// A producer-certified constraint, not proof that any particular node is
    /// initialized or valid. Union/MaybeUninit representations never inherit it.
    pub fn record_scalar_validity(&mut self, ty: TyId, validity: ScalarValidity) {
        assert!(!self.unions.contains(&ty));
        self.scalar_validity.insert(ty, validity);
    }

    pub fn scalar_validity(&self, ty: TyId) -> Option<ScalarValidity> {
        (!self.unions.contains(&ty))
            .then(|| self.scalar_validity.get(&ty).copied())
            .flatten()
    }

    pub fn scalar_valid_range(&self, ty: TyId) -> Option<ScalarValidRange> {
        match self.scalar_validity(ty)? {
            ScalarValidity::Scalar(range) => Some(range),
            ScalarValidity::Pair { .. } => None,
        }
    }

    /// Mark the actual core MaybeUninit lang item after its inner type is known.
    /// Keep union identity and field layout; this does not make it a product.
    pub fn record_maybe_uninit(&mut self, wrapper: TyId, inner: TyId) {
        self.unions.insert(wrapper);
        self.maybe_uninit.insert(wrapper, inner);
    }

    pub fn record_unsized(&mut self, ty: TyId) {
        self.unsized_types.insert(ty);
    }

    pub fn record_thin_unsized(&mut self, ty: TyId) {
        self.record_unsized(ty);
        self.thin_unsized.insert(ty);
    }

    pub fn record_static_slice_tail(&mut self, ty: TyId, field: u32) {
        self.record_unsized(ty);
        self.static_slice_tails.insert(ty, field);
    }

    pub fn record_ordering(&mut self, ty: TyId) {
        self.ordering.insert(ty);
    }

    pub fn record_vector(&mut self, ty: TyId, layout: VectorLayout) {
        self.vectors.insert(ty, layout);
    }

    pub fn record_abi_pair(&mut self, ty: TyId, fields: [FieldLayout; 2]) {
        self.abi_pairs.insert(ty, fields);
    }

    pub fn record_aggregate(&mut self, ty: TyId, layout: AggregateLayout) {
        // Bootstrap producers already carry logical fields alongside offsets.
        // Copy only those semantic facts; the view never receives the layout.
        self.record_semantic_aggregate(
            ty,
            SemanticAggregate {
                fields: layout.fields.iter().map(|field| field.ty).collect(),
                variants: layout
                    .variants
                    .iter()
                    .map(|fields| fields.iter().map(|field| field.ty).collect())
                    .collect(),
            },
        );
        self.aggregates.insert(ty, layout);
    }

    pub fn record_semantic_aggregate(&mut self, ty: TyId, aggregate: SemanticAggregate) {
        self.semantic_aggregates.insert(ty, aggregate);
    }

    pub fn record_semantic_enum(&mut self, ty: TyId, discriminants: Vec<u128>) {
        self.semantic_discriminants.insert(ty, discriminants);
    }

    pub fn record_semantic_inhabited_variants(&mut self, ty: TyId, variants: Vec<u32>) {
        self.semantic_inhabited_variants.insert(ty, variants);
    }

    pub fn semantic_aggregate(&self, ty: TyId) -> Option<&SemanticAggregate> {
        self.semantic_aggregates.get(&ty)
    }

    pub fn semantic_discriminants(&self, ty: TyId) -> Option<&[u128]> {
        self.semantic_discriminants.get(&ty).map(Vec::as_slice)
    }

    pub fn semantic(&self) -> SemanticTypes<'_> {
        SemanticTypes {
            table: &self.table,
            aggregates: &self.semantic_aggregates,
            discriminants: &self.semantic_discriminants,
            inhabited_variants: &self.semantic_inhabited_variants,
            unions: &self.unions,
            maybe_uninit: &self.maybe_uninit,
            unsized_types: &self.unsized_types,
            thin_unsized: &self.thin_unsized,
            static_slice_tails: &self.static_slice_tails,
            ordering: &self.ordering,
            vectors: &self.vectors,
        }
    }

    pub fn record_union(&mut self, ty: TyId, fields: Vec<FieldLayout>) {
        self.record_aggregate(
            ty,
            AggregateLayout {
                fields,
                variants: Vec::new(),
            },
        );
        self.unions.insert(ty);
    }

    pub fn field(&self, ty: TyId, variant: Option<u32>, index: u32) -> Option<FieldLayout> {
        let layout = self.aggregates.get(&ty)?;
        let fields = match variant {
            Some(variant) => layout.variants.get(variant as usize)?,
            None => &layout.fields,
        };
        fields.get(index as usize).copied()
    }

    pub fn intern(&mut self, shape: TypeShape, layout: Option<SizeAlign>) -> TyId {
        let layout = layout.or_else(|| {
            matches!(shape, TypeShape::PlaceAddress { .. })
                .then_some(SizeAlign { size: 8, align: 8 })
        });
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

#[cfg(test)]
mod semantic_tests {
    use super::*;
    use crate::IntegerWidth;

    #[test]
    fn single_variant_layout_does_not_make_a_struct_an_enum() {
        let mut types = RecordedTypes::new();
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            None,
        );
        let structure = types.intern(
            TypeShape::Adt {
                name: "Record".into(),
            },
            None,
        );
        let empty = types.intern(
            TypeShape::Adt {
                name: "Empty".into(),
            },
            None,
        );
        let field = FieldLayout {
            ty: byte,
            offset: 7,
        };
        types.record_aggregate(
            structure,
            AggregateLayout {
                fields: vec![field],
                variants: vec![vec![field]],
            },
        );
        types.record_aggregate(
            empty,
            AggregateLayout {
                fields: Vec::new(),
                variants: vec![Vec::new()],
            },
        );
        let semantic = types.semantic();
        assert!(!semantic.is_enum(structure));
        assert!(!semantic.is_enum(empty));
        assert_eq!(semantic.field_types(structure, None), Some(vec![byte]));
        assert_eq!(semantic.field_types(empty, None), Some(Vec::new()));
        assert_eq!(semantic.variant_count(structure), None);
        assert_eq!(semantic.field_type(structure, Some(0), 0), None);
    }

    #[test]
    fn semantic_fields_exist_without_any_layout() {
        let mut types = RecordedTypes::new();
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            None,
        );
        let structure = types.intern(
            TypeShape::Adt {
                name: "Unlaid".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            structure,
            SemanticAggregate {
                fields: vec![byte],
                variants: Vec::new(),
            },
        );
        assert!(types.aggregates.is_empty());
        assert_eq!(types.size_align(structure), None);
        assert_eq!(types.semantic().field_type(structure, None, 0), Some(byte));
    }

    #[test]
    fn logical_discriminants_are_independent_of_niche_encoding() {
        let mut types = RecordedTypes::new();
        let ty = types.intern(
            TypeShape::Adt {
                name: "Choice".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            ty,
            SemanticAggregate {
                fields: Vec::new(),
                variants: vec![Vec::new(), Vec::new()],
            },
        );
        types.enums.insert(
            ty,
            EnumLayout {
                tag: None,
                encoding: TagEncoding::Niche {
                    untagged_variant: 1,
                    niche_variants: 0..=0,
                    niche_start: 0,
                },
            },
        );
        assert!(!types.semantic().is_enum(ty));
        assert_eq!(types.semantic().discriminant(ty, 0), None);
        types.record_semantic_enum(ty, vec![11, 29]);
        assert!(types.semantic().is_enum(ty));
        assert_eq!(types.semantic().variant_count(ty), Some(2));
        assert_eq!(types.semantic().discriminant(ty, 0), Some(11));
        assert_eq!(types.semantic().discriminant(ty, 1), Some(29));
        assert_eq!(types.semantic().field_types(ty, Some(1)), Some(Vec::new()));
        assert_eq!(types.semantic().field_types(ty, None), None);
        assert_eq!(types.semantic().discriminant(ty, 2), None);
    }
}
