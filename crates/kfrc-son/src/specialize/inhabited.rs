//! Boundary-only proof that no valid value can cross a source signature.
//! Unknown validity stays unknown; pointers and opaque MaybeUninit do not
//! inherit the validity of their pointee/inner storage.
use kfrc_body::{RecordedTypes, TyId, TypeOracle, TypeShape};
use std::collections::HashSet;

pub(super) fn impossible(types: &RecordedTypes, ty: TyId) -> bool {
    fn visit(types: &RecordedTypes, ty: TyId, active: &mut HashSet<TyId>) -> bool {
        if types.semantic().maybe_uninit_inner(ty).is_some() || !active.insert(ty) {
            return false;
        }
        let answer = if types.semantic().is_enum(ty) {
            types.semantic().variant_count(ty) == Some(0)
                || types.semantic().inhabited_variants(ty) == Some(&[])
        } else {
            match types.shape(ty) {
                TypeShape::Never => true,
                TypeShape::Array { element, length } => {
                    *length != 0 && visit(types, *element, active)
                }
                TypeShape::Tuple(fields) => fields.iter().any(|field| visit(types, *field, active)),
                TypeShape::Adt { .. } if !types.semantic().is_union(ty) => {
                    types.semantic_aggregate(ty).is_some_and(|aggregate| {
                        aggregate
                            .fields
                            .iter()
                            .any(|field| visit(types, *field, active))
                    })
                }
                _ => false,
            }
        };
        active.remove(&ty);
        answer
    }
    visit(types, ty, &mut HashSet::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kfrc_body::{Mutability, SemanticAggregate};
    #[test]
    fn proof_obeys_storage_and_zero_length_boundaries() {
        let mut types = RecordedTypes::new();
        let never = types.intern(TypeShape::Never, None);
        let empty = types.intern(
            TypeShape::Array {
                element: never,
                length: 0,
            },
            None,
        );
        let nonempty = types.intern(
            TypeShape::Array {
                element: never,
                length: 1,
            },
            None,
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: never,
            },
            None,
        );
        let address = types.intern(TypeShape::PlaceAddress { pointee: never }, None);
        let maybe = types.intern(
            TypeShape::Adt {
                name: "opaque".into(),
            },
            None,
        );
        types.record_maybe_uninit(maybe, never);
        let product = types.intern(TypeShape::Tuple(vec![empty, never]), None);
        let unknown = types.intern(
            TypeShape::Adt {
                name: "unknown".into(),
            },
            None,
        );
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "empty enum".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            enumeration,
            SemanticAggregate {
                fields: vec![],
                variants: vec![],
            },
        );
        types.record_semantic_enum(enumeration, vec![]);
        for ty in [empty, pointer, address, maybe, unknown] {
            assert!(!impossible(&types, ty), "{ty:?}");
        }
        for ty in [never, nonempty, product, enumeration] {
            assert!(impossible(&types, ty), "{ty:?}");
        }
    }
}
