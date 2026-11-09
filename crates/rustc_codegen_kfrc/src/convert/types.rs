//! Record Rust type shapes and rustc's target layout answers.

use super::{Converter, convert_mutability};
use kfrc_body::{
    AggregateLayout, FieldLayout, IntegerWidth, Mutability, SizeAlign, TyId, TypeShape,
};
use rustc_middle::ty::layout::PrimitiveExt;
use rustc_middle::ty::{self, Ty, TypingEnv};

impl<'tcx> Converter<'tcx> {
    pub(super) fn ty(&mut self, ty: Ty<'tcx>) -> TyId {
        if let Some(id) = self.types.get(&ty) {
            return *id;
        }
        let shape = match ty.kind() {
            ty::Bool => TypeShape::Bool,
            ty::Char => TypeShape::Char,
            ty::Int(int) => TypeShape::Int {
                width: integer_width(int.bit_width()),
                signed: true,
            },
            ty::Uint(uint) => TypeShape::Int {
                width: integer_width(uint.bit_width()),
                signed: false,
            },
            ty::Float(float) => TypeShape::Float {
                bits: float.bit_width() as u8,
            },
            ty::Tuple(elements) if elements.is_empty() => TypeShape::Unit,
            ty::Tuple(elements) => {
                TypeShape::Tuple(elements.iter().map(|element| self.ty(element)).collect())
            }
            ty::Array(element, length) => TypeShape::Array {
                element: self.ty(*element),
                length: length.try_to_target_usize(self.tcx).unwrap_or_else(|| {
                    self.tcx.dcx().fatal(format!(
                        "kfrc: array type `{ty}` has no monomorphic target length"
                    ))
                }),
            },
            ty::Slice(element) => TypeShape::Slice {
                element: self.ty(*element),
            },
            ty::Str => TypeShape::Str,
            ty::Never => TypeShape::Never,
            ty::Ref(_, pointee, mutability) => TypeShape::Reference {
                mutability: convert_mutability(*mutability),
                pointee: self.ty(*pointee),
            },
            ty::RawPtr(pointee, mutability) => TypeShape::RawPointer {
                mutability: convert_mutability(*mutability),
                pointee: self.ty(*pointee),
            },
            ty::Adt(..) | ty::Closure(..) => TypeShape::Adt {
                name: crate::type_identity::name(self.tcx, ty),
            },
            ty::FnPtr(..) => TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: self.ty(self.tcx.types.unit),
            },
            ty::FnDef(..) => TypeShape::FunctionItem {
                name: crate::type_identity::name(self.tcx, ty),
            },
            _ => TypeShape::Other {
                name: crate::type_identity::name(self.tcx, ty),
            },
        };
        let rust_layout = self
            .tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
            .ok();
        let layout = rust_layout
            .filter(|layout| layout.is_sized())
            .map(|layout| SizeAlign {
                size: layout.size.bytes(),
                align: layout.align.abi.bytes(),
            });
        let id = self.builder.types().intern(shape, layout);
        // Intern before traversing fields: recursive ADTs can point back to
        // this type through a reference or a Box.
        self.types.insert(ty, id);
        if scalar_validity_identity_preserved(ty)
            && !matches!(ty.kind(), ty::Adt(def, _) if def.is_union())
            && let Some(validity) = rust_layout.and_then(|layout| scalar_validity(self.tcx, layout.backend_repr))
        {
            self.builder.types().record_scalar_validity(id, validity);
        }
        if let ty::Adt(def, _) = ty.kind()
            && self.tcx.lang_items().ordering_enum() == Some(def.did())
        {
            self.builder.types().record_ordering(id);
        }
        if let ty::Adt(def, args) = ty.kind()
            && self.tcx.lang_items().maybe_uninit() == Some(def.did())
        {
            let inner = self.ty(args.type_at(0));
            self.builder.types().record_maybe_uninit(id, inner);
        }
        if let ty::Adt(def, args) = ty.kind()
            && def.is_enum()
        {
            // Inhabitation is a type fact, not a consequence of the selected
            // tag encoding. In particular Option<Infallible> has two declared
            // variants but only None can be constructed, even when no ABI
            // value is passed for its zero-sized representation.
            let variants = def
                .variants()
                .iter_enumerated()
                .filter_map(|(index, variant)| {
                    variant
                        .inhabited_predicate(self.tcx, *def)
                        .instantiate(self.tcx, args)
                        .apply_ignore_module(self.tcx, TypingEnv::fully_monomorphized())
                        .then_some(index.as_u32())
                })
                .collect();
            self.builder
                .types()
                .record_semantic_inhabited_variants(id, variants);
        }
        if let Some(layout) = rust_layout {
            if !layout.is_sized() {
                self.builder.types().record_unsized(id);
                if matches!(structural_tail(self.tcx, ty).kind(), ty::Foreign(_)) {
                    // Extern types (and structs ending in one) are unsized
                    // but carry unit metadata, so their Rust pointers are thin.
                    self.builder.types().record_thin_unsized(id);
                }
                if let ty::Adt(def, _) = ty.kind()
                    && def.is_struct()
                    && has_static_slice_tail(self.tcx, ty)
                {
                    let tail = def.non_enum_variant().fields.len() - 1;
                    self.builder
                        .types()
                        .record_static_slice_tail(id, tail as u32);
                }
            }
            if let rustc_abi::BackendRepr::SimdVector { element, count } = layout.backend_repr {
                let element = self.ty(element.primitive().to_ty(self.tcx));
                let lanes = u32::try_from(count)
                    .unwrap_or_else(|_| self.tcx.dcx().fatal("kfrc: SIMD lane count exceeds u32"));
                self.builder
                    .types()
                    .record_vector(id, kfrc_body::VectorLayout { element, lanes });
            }
            let cx =
                rustc_middle::ty::layout::LayoutCx::new(self.tcx, TypingEnv::fully_monomorphized());
            let mut recorded = AggregateLayout {
                fields: Vec::new(),
                variants: Vec::new(),
            };
            match ty.kind() {
                ty::Tuple(..) | ty::Closure(..) => {
                    for index in 0..layout.fields.count() {
                        let field = layout.field(&cx, index);
                        recorded.fields.push(FieldLayout {
                            ty: self.ty(field.ty),
                            offset: layout.fields.offset(index).bytes(),
                        });
                    }
                }
                ty::Adt(def, _) if def.is_union() => {
                    let mut fields = Vec::new();
                    for index in 0..layout.fields.count() {
                        let field = layout.field(&cx, index);
                        fields.push(FieldLayout {
                            ty: self.ty(field.ty),
                            offset: layout.fields.offset(index).bytes(),
                        });
                    }
                    self.builder.types().record_union(id, fields);
                }
                ty::Adt(def, _) => {
                    for (variant, _) in def.variants().iter_enumerated() {
                        let variant_layout = layout.for_variant(&cx, variant);
                        let mut fields = Vec::new();
                        for index in 0..variant_layout.fields.count() {
                            let field = variant_layout.field(&cx, index);
                            fields.push(FieldLayout {
                                ty: self.ty(field.ty),
                                offset: variant_layout.fields.offset(index).bytes(),
                            });
                        }
                        recorded.variants.push(fields);
                    }
                    if def.is_struct() {
                        recorded.fields = recorded.variants[0].clone();
                    }
                }
                ty::Ref(..) | ty::RawPtr(..) if layout.size.bytes() == 16 => {
                    // rustc's field(0) returns a thin layout while retaining
                    // the original wide pointer Ty (`ty/layout.rs`). Our
                    // oracle keys layouts by TyId, so use a canonical thin
                    // pointer type and retain rustc's actual field offset.
                    let ptr_ty = self.ty(Ty::new_imm_ptr(self.tcx, self.tcx.types.unit));
                    // rustc gives slices a usize and dyn objects a vtable
                    // pointer. Preserve that distinction for metadata loads.
                    let meta_ty = self.ty(layout.field(&cx, 1).ty);
                    recorded.fields.push(FieldLayout {
                        ty: ptr_ty,
                        offset: layout.fields.offset(0).bytes(),
                    });
                    recorded.fields.push(FieldLayout {
                        ty: meta_ty,
                        offset: layout.fields.offset(1).bytes(),
                    });
                }
                _ => {}
            }
            if !matches!(ty.kind(), ty::Adt(def, _) if def.is_union())
                && (!recorded.fields.is_empty()
                    || !recorded.variants.is_empty()
                    || matches!(ty.kind(), ty::Tuple(..) | ty::Closure(..) | ty::Adt(..)))
            {
                self.builder.types().record_aggregate(id, recorded);
            }
            if let rustc_abi::BackendRepr::ScalarPair(first, second) = layout.backend_repr {
                let fields = [
                    FieldLayout {
                        ty: self.ty(first.primitive().to_ty(self.tcx)),
                        offset: 0,
                    },
                    FieldLayout {
                        ty: self.ty(second.primitive().to_ty(self.tcx)),
                        offset: first
                            .size(&self.tcx)
                            .align_to(second.align(&self.tcx).abi)
                            .bytes(),
                    },
                ];
                self.builder.types().record_abi_pair(id, fields);
            }
            if let Some(description) = crate::enum_layout::enum_description(self.tcx, ty)
                .unwrap_or_else(|error| self.tcx.dcx().fatal(format!("kfrc enum layout: {error}")))
            {
                use crate::enum_layout::EnumEncoding;
                // Source-level discriminants are independent of the physical
                // tag/niche encoding. The semantic graph reads this table.
                self.builder
                    .types()
                    .record_semantic_enum(id, description.discriminants.clone());
                let tag = description.tag.map(|tag| FieldLayout {
                    ty: self.ty(tag.ty),
                    offset: tag.offset,
                });
                let encoding = match description.encoding {
                    EnumEncoding::Empty => kfrc_body::TagEncoding::Empty,
                    EnumEncoding::Single { variant } => kfrc_body::TagEncoding::Single {
                        variant,
                        discriminant: description.discriminants[variant as usize],
                    },
                    EnumEncoding::Direct => kfrc_body::TagEncoding::Direct {
                        discriminants: description.discriminants,
                    },
                    EnumEncoding::Niche {
                        untagged_variant,
                        niche_variants,
                        niche_start,
                    } => kfrc_body::TagEncoding::Niche {
                        untagged_variant,
                        niche_variants,
                        niche_start,
                    },
                };
                self.builder
                    .types()
                    .enums
                    .insert(id, kfrc_body::EnumLayout { tag, encoding });
            }
        }
        id
    }
}

fn integer_width(bits: Option<u64>) -> IntegerWidth {
    match bits {
        Some(8) => IntegerWidth::W8,
        Some(16) => IntegerWidth::W16,
        Some(32) => IntegerWidth::W32,
        Some(64) => IntegerWidth::W64,
        Some(128) => IntegerWidth::W128,
        _ => IntegerWidth::Pointer,
    }
}

/// FnPtr currently uses the same structural TyId as nullable `*const ()`.
/// Its nonnull validity cannot be attached to that erased identity, including
/// when a tuple/array exposes the same erasure in its own scalar lanes.
/// Nominal ADTs/closures/opaque types keep the complete native-type fingerprint.
/// Pointer/reference validity describes the pointer itself, not its pointee.
pub(super) fn scalar_validity_identity_preserved(ty: Ty<'_>) -> bool {
    match ty.kind() {
        ty::FnPtr(..) => false,
        ty::Tuple(fields) => fields.iter().all(scalar_validity_identity_preserved),
        ty::Array(element, _) => scalar_validity_identity_preserved(*element),
        _ => true,
    }
}

/// Keep lane constraints on the nominal type, never on its primitive ABI TyId.
pub(super) fn scalar_validity(
    tcx: ty::TyCtxt<'_>,
    repr: rustc_abi::BackendRepr,
) -> Option<kfrc_body::ScalarValidity> {
    use kfrc_body::{ScalarValidRange, ScalarValidity};
    use rustc_abi::{BackendRepr, Scalar};
    let restricted = |scalar: Scalar| {
        let Scalar::Initialized { valid_range, .. } = scalar else { return None };
        let bits = u8::try_from(scalar.size(&tcx).bits()).ok()?;
        let range = ScalarValidRange::new(bits, valid_range.start, valid_range.end)?;
        (!range.is_full()).then_some(range)
    };
    match repr {
        BackendRepr::Scalar(scalar) => restricted(scalar).map(ScalarValidity::Scalar),
        BackendRepr::ScalarPair(first, second) => {
            let first_range = restricted(first);
            let second_range = restricted(second);
            (first_range.is_some() || second_range.is_some()).then_some(ScalarValidity::Pair {
                first: first_range,
                second: second_range,
                second_offset: first.size(&tcx).align_to(second.align(&tcx).abi).bytes(),
            })
        }
        _ => None,
    }
}

/// Slice/str tails have target-static alignment and propagate usize metadata.
/// A dyn tail can require runtime alignment and is deliberately not certified.
fn has_static_slice_tail<'tcx>(tcx: ty::TyCtxt<'tcx>, tail: Ty<'tcx>) -> bool {
    matches!(structural_tail(tcx, tail).kind(), ty::Slice(_) | ty::Str)
}

fn structural_tail<'tcx>(tcx: ty::TyCtxt<'tcx>, mut tail: Ty<'tcx>) -> Ty<'tcx> {
    loop {
        match tail.kind() {
            ty::Adt(def, args) if def.is_struct() => {
                let Some(field) = def.non_enum_variant().fields.iter().last() else {
                    return tail;
                };
                tail = tcx.normalize_erasing_regions(
                    TypingEnv::fully_monomorphized(),
                    field.ty(tcx, args),
                );
            }
            _ => return tail,
        }
    }
}
