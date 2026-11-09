//! Import complete recorded type facts; TyIds are local to each graph.
use super::InlineSkip;
use crate::ValueType;
use kfrc_body::{
    AggregateLayout, FieldLayout, RecordedTypes, SemanticAggregate, TyId, TypeOracle, TypeShape,
};
use std::collections::HashMap;

pub(super) struct TypeImport<'a> {
    source: &'a RecordedTypes,
    target: &'a mut RecordedTypes,
    mapped: HashMap<TyId, TyId>,
    /// Fresh nominal shells have no facts yet; pre-existing types do.
    existing_types: usize,
}
impl<'a> TypeImport<'a> {
    pub fn new(source: &'a RecordedTypes, target: &'a mut RecordedTypes) -> Self {
        let mut mapped = HashMap::new();
        let existing_types = target.table.len();
        // Nominal shells break recursive ADT -> pointer -> ADT cycles. Facts
        // are validated below; the name is not considered sufficient proof.
        for i in 0..source.table.len() {
            let id = TyId(i as u32);
            if matches!(source.shape(id), TypeShape::Adt { .. }) {
                mapped.insert(id, target.table.intern(source.shape(id).clone()));
            }
        }
        Self {
            source,
            target,
            mapped,
            existing_types,
        }
    }
    pub fn value_type(&mut self, ty: &ValueType) -> Result<ValueType, InlineSkip> {
        Ok(match ty {
            ValueType::Rust(ty) => ValueType::Rust(self.ty(*ty)?),
            ValueType::LayoutValue(_) => return Err(InlineSkip::Unsupported),
            ValueType::Control => ValueType::Control,
            ValueType::Effect => ValueType::Effect,
            ValueType::Witness => ValueType::Witness,
            ValueType::Tuple(fields) => ValueType::Tuple(
                fields
                    .iter()
                    .map(|ty| self.value_type(ty))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
    fn ty(&mut self, source: TyId) -> Result<TyId, InlineSkip> {
        if let Some(&ty) = self.mapped.get(&source) {
            return Ok(ty);
        }
        let shape = match self.source.shape(source).clone() {
            TypeShape::Tuple(fields) => TypeShape::Tuple(self.ids(&fields)?),
            TypeShape::Array { element, length } => TypeShape::Array {
                element: self.ty(element)?,
                length,
            },
            TypeShape::Slice { element } => TypeShape::Slice {
                element: self.ty(element)?,
            },
            TypeShape::Reference {
                mutability,
                pointee,
            } => TypeShape::Reference {
                mutability,
                pointee: self.ty(pointee)?,
            },
            TypeShape::RawPointer {
                mutability,
                pointee,
            } => TypeShape::RawPointer {
                mutability,
                pointee: self.ty(pointee)?,
            },
            TypeShape::PlaceAddress { pointee } => TypeShape::PlaceAddress {
                pointee: self.ty(pointee)?,
            },
            shape => shape,
        };
        let target = self.target.table.intern(shape);
        self.mapped.insert(source, target);
        Ok(target)
    }
    fn ids(&mut self, ids: &[TyId]) -> Result<Vec<TyId>, InlineSkip> {
        ids.iter().map(|ty| self.ty(*ty)).collect()
    }
    fn field(&mut self, field: FieldLayout) -> Result<FieldLayout, InlineSkip> {
        Ok(FieldLayout {
            ty: self.ty(field.ty)?,
            offset: field.offset,
        })
    }
    fn fields(&mut self, fields: &[FieldLayout]) -> Result<Vec<FieldLayout>, InlineSkip> {
        fields.iter().map(|field| self.field(*field)).collect()
    }

    pub fn finish(mut self) -> Result<(), InlineSkip> {
        // Import the oracle closure, including recursively named nominal fields.
        for i in 0..self.source.table.len() {
            self.ty(TyId(i as u32))?;
        }
        for i in 0..self.source.table.len() {
            let source = TyId(i as u32);
            let target = self.mapped[&source];
            let size = self.source.size_align(source);
            let aggregate = self
                .source
                .aggregates
                .get(&source)
                .cloned()
                .map(|layout| {
                    Ok::<_, InlineSkip>(AggregateLayout {
                        fields: self.fields(&layout.fields)?,
                        variants: layout
                            .variants
                            .iter()
                            .map(|fields| self.fields(fields))
                            .collect::<Result<_, _>>()?,
                    })
                })
                .transpose()?;
            let semantic = self
                .source
                .semantic_aggregate(source)
                .cloned()
                .map(|aggregate| {
                    Ok::<_, InlineSkip>(SemanticAggregate {
                        fields: self.ids(&aggregate.fields)?,
                        variants: aggregate
                            .variants
                            .iter()
                            .map(|fields| self.ids(fields))
                            .collect::<Result<_, _>>()?,
                    })
                })
                .transpose()?;
            let discriminants = self
                .source
                .semantic_discriminants(source)
                .map(<[u128]>::to_vec);
            let inhabited = self
                .source
                .semantic()
                .inhabited_variants(source)
                .map(<[u32]>::to_vec);
            let scalar_validity = self.source.scalar_validity(source);
            let encoding = self
                .source
                .enums
                .get(&source)
                .cloned()
                .map(|mut layout| {
                    layout.tag = layout.tag.map(|field| self.field(field)).transpose()?;
                    Ok::<_, InlineSkip>(layout)
                })
                .transpose()?;
            let pair = self
                .source
                .abi_pairs
                .get(&source)
                .copied()
                .map(|fields| Ok::<_, InlineSkip>([self.field(fields[0])?, self.field(fields[1])?]))
                .transpose()?;
            let vector = self
                .source
                .vectors
                .get(&source)
                .copied()
                .map(|mut vector| {
                    vector.element = self.ty(vector.element)?;
                    Ok::<_, InlineSkip>(vector)
                })
                .transpose()?;
            let union = self.source.unions.contains(&source);
            let maybe_uninit = self
                .source
                .semantic()
                .maybe_uninit_inner(source)
                .map(|inner| self.ty(inner))
                .transpose()?;
            let old_maybe_uninit = self.target.semantic().maybe_uninit_inner(target);
            let is_unsized = self.source.semantic().is_unsized(source);
            let is_thin_unsized = self.source.semantic().is_thin_unsized(source);
            let ordering = self.source.semantic().is_ordering(source);
            let static_slice_tail = self.source.semantic().static_slice_tail(source);
            let existing = (target.0 as usize) < self.existing_types;
            // Existing facts are compared before any mutator can overwrite them.
            if conflicts(self.target.size_align(target).as_ref(), size.as_ref())
                || conflicts(self.target.aggregates.get(&target), aggregate.as_ref())
                || conflicts(self.target.semantic_aggregate(target), semantic.as_ref())
                || conflicts(
                    self.target.semantic_discriminants(target),
                    discriminants.as_deref(),
                )
                || conflicts(
                    self.target.semantic().inhabited_variants(target),
                    inhabited.as_deref(),
                )
                || conflicts(self.target.enums.get(&target), encoding.as_ref())
                || conflicts(self.target.scalar_validity(target).as_ref(), scalar_validity.as_ref())
                || conflicts(self.target.abi_pairs.get(&target), pair.as_ref())
                || conflicts(self.target.vectors.get(&target), vector.as_ref())
                || conflicts(old_maybe_uninit.as_ref(), maybe_uninit.as_ref())
                || (existing
                    && (self.target.semantic().is_unsized(target) != is_unsized
                        || self.target.semantic().is_thin_unsized(target) != is_thin_unsized
                        || self.target.semantic().is_ordering(target) != ordering
                        || self.target.semantic().static_slice_tail(target) != static_slice_tail))
                || (maybe_uninit.is_some()
                    && old_maybe_uninit.is_none()
                    && self.target.unions.contains(&target))
                || (self.target.unions.contains(&target) && !union)
                || (union
                    && self.target.semantic_aggregate(target).is_some()
                    && !self.target.unions.contains(&target))
            {
                return Err(InlineSkip::TypeConflict);
            }
            if self.target.layouts.len() <= target.0 as usize {
                self.target.layouts.resize(target.0 as usize + 1, None);
            }
            self.target.layouts[target.0 as usize] = size;
            if let Some(aggregate) = aggregate {
                self.target.record_aggregate(target, aggregate);
            }
            if let Some(semantic) = semantic {
                self.target.record_semantic_aggregate(target, semantic);
            }
            if let Some(discriminants) = discriminants {
                self.target.record_semantic_enum(target, discriminants);
            }
            if let Some(inhabited) = inhabited {
                self.target
                    .record_semantic_inhabited_variants(target, inhabited);
            }
            if let Some(validity) = scalar_validity {
                self.target.record_scalar_validity(target, validity);
            }
            if let Some(encoding) = encoding {
                self.target.enums.insert(target, encoding);
            }
            if let Some(pair) = pair {
                self.target.abi_pairs.insert(target, pair);
            }
            if let Some(vector) = vector {
                self.target.vectors.insert(target, vector);
            }
            if union {
                self.target.unions.insert(target);
            }
            if let Some(inner) = maybe_uninit {
                self.target.record_maybe_uninit(target, inner);
            }
            if is_unsized {
                self.target.record_unsized(target);
            }
            if is_thin_unsized {
                self.target.record_thin_unsized(target);
            }
            if ordering {
                self.target.record_ordering(target);
            }
            if let Some(field) = static_slice_tail {
                self.target.record_static_slice_tail(target, field);
            }
        }
        Ok(())
    }
}
fn conflicts<T: PartialEq + ?Sized>(old: Option<&T>, new: Option<&T>) -> bool {
    old.is_some_and(|old| Some(old) != new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kfrc_body::IntegerWidth;

    #[test]
    fn arbitrary_missing_bool_range_is_not_filled_during_import() {
        use kfrc_body::{ScalarValidRange, ScalarValidity, SizeAlign};
        let mut source = RecordedTypes::new();
        source.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let mut target = RecordedTypes::new();
        let ty = target.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        target.record_scalar_validity(ty, ScalarValidity::Scalar(ScalarValidRange::new(8, 0, 1).unwrap()));
        assert_eq!(TypeImport::new(&source, &mut target).finish(), Err(InlineSkip::TypeConflict));
    }

    #[test]
    fn scalar_validity_import_preserves_nominal_identity_and_rejects_conflicts() {
        use kfrc_body::{ScalarValidRange, ScalarValidity};
        let nominal = TypeShape::Adt { name: "RestrictedWord#same-identity".into() };
        let mut source = RecordedTypes::new();
        let source_ty = source.intern(nominal.clone(), None);
        let validity = ScalarValidity::Scalar(ScalarValidRange::new(32, 1, u32::MAX as u128).unwrap());
        source.record_scalar_validity(source_ty, validity);
        let mut target = RecordedTypes::new();
        target.intern(TypeShape::Bool, None);
        let mut import = TypeImport::new(&source, &mut target);
        let ValueType::Rust(imported) = import.value_type(&ValueType::Rust(source_ty)).unwrap() else { unreachable!() };
        import.finish().unwrap();
        assert_eq!(target.scalar_validity(imported), Some(validity));
        let mut conflict = RecordedTypes::new();
        let ty = conflict.intern(nominal, None);
        conflict.record_scalar_validity(ty, ScalarValidity::Scalar(ScalarValidRange::new(32, 2, u32::MAX as u128).unwrap()));
        assert_eq!(TypeImport::new(&source, &mut conflict).finish(), Err(InlineSkip::TypeConflict));
        let mut different = RecordedTypes::new();
        let other = different.intern(TypeShape::Adt { name: "DifferentWord#other-identity".into() }, None);
        TypeImport::new(&source, &mut different).finish().unwrap();
        assert_eq!(different.scalar_validity(other), None);
    }

    fn wrapper(types: &mut RecordedTypes, width: IntegerWidth, marked: bool) -> (TyId, TyId) {
        let inner = types.intern(
            TypeShape::Int {
                width,
                signed: false,
            },
            None,
        );
        let wrapper = types.intern(
            TypeShape::Adt {
                name: "core::mem::MaybeUninit<Payload>".into(),
            },
            None,
        );
        types.unions.insert(wrapper);
        if marked {
            types.record_maybe_uninit(wrapper, inner);
        }
        (wrapper, inner)
    }

    #[test]
    fn maybe_uninit_import_remaps_inner_type_and_keeps_union_identity() {
        let mut source = RecordedTypes::new();
        let (source_wrapper, source_inner) = wrapper(&mut source, IntegerWidth::W64, true);
        let mut target = RecordedTypes::new();
        target.intern(TypeShape::Bool, None);
        target.intern(TypeShape::Unit, None);
        let mut import = TypeImport::new(&source, &mut target);
        let ValueType::Rust(target_wrapper) =
            import.value_type(&ValueType::Rust(source_wrapper)).unwrap()
        else {
            unreachable!();
        };
        import.finish().unwrap();
        let target_inner = target
            .semantic()
            .maybe_uninit_inner(target_wrapper)
            .unwrap();
        assert_ne!(source_inner, target_inner);
        assert_eq!(target.shape(target_inner), source.shape(source_inner));
        assert!(target.unions.contains(&target_wrapper));
        assert!(!target.semantic().is_product(target_wrapper));
    }

    #[test]
    fn maybe_uninit_import_rejects_inner_conflicts_and_marker_changes() {
        for (source_marked, target_marked, target_width) in [
            (true, true, IntegerWidth::W32),
            (true, false, IntegerWidth::W64),
            (false, true, IntegerWidth::W64),
        ] {
            let mut source = RecordedTypes::new();
            wrapper(&mut source, IntegerWidth::W64, source_marked);
            let mut target = RecordedTypes::new();
            wrapper(&mut target, target_width, target_marked);
            assert_eq!(
                TypeImport::new(&source, &mut target).finish(),
                Err(InlineSkip::TypeConflict)
            );
        }
    }

    #[test]
    fn place_addresses_and_semantic_markers_survive_independent_type_tables() {
        let mut source = RecordedTypes::new();
        let byte = source.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            None,
        );
        let slice = source.intern(TypeShape::Slice { element: byte }, None);
        let owner = source.intern(
            TypeShape::Adt {
                name: "stable-id:tail-owner".into(),
            },
            None,
        );
        source.record_semantic_aggregate(
            owner,
            SemanticAggregate {
                fields: vec![byte, slice],
                variants: vec![],
            },
        );
        source.record_static_slice_tail(owner, 1);
        let address = source.intern(TypeShape::PlaceAddress { pointee: owner }, None);
        let raw = source.intern(
            TypeShape::RawPointer {
                mutability: kfrc_body::Mutability::Mut,
                pointee: owner,
            },
            None,
        );
        let ordering = source.intern(
            TypeShape::Adt {
                name: "stable-id:cmp-result".into(),
            },
            None,
        );
        source.record_semantic_aggregate(
            ordering,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![], vec![], vec![]],
            },
        );
        source.record_semantic_enum(ordering, vec![u128::MAX, 0, 1]);
        source.record_ordering(ordering);

        let mut target = RecordedTypes::new();
        target.intern(TypeShape::Unit, None);
        target.intern(TypeShape::Bool, None);
        target.intern(TypeShape::Char, None);
        let mut import = TypeImport::new(&source, &mut target);
        let mapped_owner = import.ty(owner).unwrap();
        let mapped_address = import.ty(address).unwrap();
        let mapped_raw = import.ty(raw).unwrap();
        let mapped_ordering = import.ty(ordering).unwrap();
        import.finish().unwrap();
        assert_ne!(owner, mapped_owner);
        assert_eq!(
            target.shape(mapped_address),
            &TypeShape::PlaceAddress {
                pointee: mapped_owner
            }
        );
        assert_eq!(
            target.semantic().thin_pointee(mapped_address),
            Some(mapped_owner)
        );
        assert_eq!(target.semantic().thin_pointee(mapped_raw), None);
        assert!(target.semantic().is_unsized(mapped_owner));
        assert_eq!(target.semantic().static_slice_tail(mapped_owner), Some(1));
        let mapped_slice = target.semantic().field_type(mapped_owner, None, 1).unwrap();
        assert!(target.semantic().is_unsized(mapped_slice));
        assert!(target.semantic().is_ordering(mapped_ordering));
        TypeImport::new(&source, &mut target).finish().unwrap();
    }

    #[test]
    fn foreign_unsized_pointers_remain_thin_after_import() {
        let mut source = RecordedTypes::new();
        let foreign = source.intern(
            TypeShape::Adt {
                name: "stable-id:foreign".into(),
            },
            None,
        );
        source.record_thin_unsized(foreign);
        let pointer = source.intern(
            TypeShape::RawPointer {
                mutability: kfrc_body::Mutability::Mut,
                pointee: foreign,
            },
            None,
        );
        let mut target = RecordedTypes::new();
        target.intern(TypeShape::Unit, None);
        let mut import = TypeImport::new(&source, &mut target);
        let mapped_foreign = import.ty(foreign).unwrap();
        let mapped_pointer = import.ty(pointer).unwrap();
        import.finish().unwrap();
        assert_ne!(foreign, mapped_foreign);
        assert!(target.semantic().is_unsized(mapped_foreign));
        assert!(target.semantic().is_thin_unsized(mapped_foreign));
        assert!(!target.semantic().requires_pointer_metadata(mapped_foreign));
        assert_eq!(
            target.semantic().thin_pointee(mapped_pointer),
            Some(mapped_foreign)
        );
        TypeImport::new(&source, &mut target).finish().unwrap();
    }

    #[test]
    fn recursive_place_address_remaps_its_nominal_shell() {
        let mut source = RecordedTypes::new();
        let node = source.intern(
            TypeShape::Adt {
                name: "Node".into(),
            },
            None,
        );
        let next = source.intern(TypeShape::PlaceAddress { pointee: node }, None);
        source.record_semantic_aggregate(
            node,
            SemanticAggregate {
                fields: vec![next],
                variants: vec![],
            },
        );
        let mut target = RecordedTypes::new();
        target.intern(TypeShape::Unit, None);
        let mut import = TypeImport::new(&source, &mut target);
        let mapped = import.ty(node).unwrap();
        import.finish().unwrap();
        let next = target.semantic().field_type(mapped, None, 0).unwrap();
        assert_eq!(
            target.shape(next),
            &TypeShape::PlaceAddress { pointee: mapped }
        );
    }

    #[test]
    fn semantic_markers_conflict_symmetrically_on_existing_nominal_types() {
        for kind in ["ordering", "unsized", "thin-unsized", "slice-tail"] {
            for (source_marked, target_marked) in [(false, true), (true, false)] {
                let make = |marked| {
                    let mut types = RecordedTypes::new();
                    let owner = types.intern(
                        TypeShape::Adt {
                            name: "StableNominal".into(),
                        },
                        None,
                    );
                    if marked {
                        match kind {
                            "ordering" => types.record_ordering(owner),
                            "unsized" => types.record_unsized(owner),
                            "thin-unsized" => types.record_thin_unsized(owner),
                            _ => types.record_static_slice_tail(owner, 0),
                        }
                    }
                    types
                };
                let source = make(source_marked);
                let mut target = make(target_marked);
                assert_eq!(
                    TypeImport::new(&source, &mut target).finish(),
                    Err(InlineSkip::TypeConflict),
                    "{kind}: source={source_marked}, target={target_marked}"
                );
            }
        }
        let mut source = RecordedTypes::new();
        let owner = source.intern(
            TypeShape::Adt {
                name: "Tail".into(),
            },
            None,
        );
        source.record_static_slice_tail(owner, 0);
        let mut target = source.clone();
        target.record_static_slice_tail(owner, 1);
        assert_eq!(
            TypeImport::new(&source, &mut target).finish(),
            Err(InlineSkip::TypeConflict)
        );
    }
}
