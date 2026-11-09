//! Record enum tag encoding from rustc's actual target layout.
//!
//! The logical discriminant type (`repr().discr_type()`) can be wider than the
//! stored tag. The stored tag's signedness and width must therefore come from
//! `Variants::Multiple::tag`, including when a niche lives inside a payload.

use std::ops::RangeInclusive;

use rustc_abi::{Integer, Primitive, TagEncoding, Variants};
use rustc_middle::ty::{self, Ty, TyCtxt, TypingEnv};

pub struct TagDescription<'tcx> {
    /// The integer type used to read/write the tag. Pointer niches use usize
    /// as their raw representation; no pointer provenance is interpreted.
    pub ty: Ty<'tcx>,
    pub offset: u64,
}

pub enum EnumEncoding {
    Empty,
    Single {
        variant: u32,
    },
    Direct,
    Niche {
        untagged_variant: u32,
        niche_variants: RangeInclusive<u32>,
        niche_start: u128,
    },
}

pub struct EnumDescription<'tcx> {
    /// Empty and single-variant layouts have no tag storage.
    pub tag: Option<TagDescription<'tcx>>,
    pub encoding: EnumEncoding,
    /// One logical source discriminant per declared variant. These are rustc's
    /// raw bits, including negative explicit values, before any tag truncation.
    pub discriminants: Vec<u128>,
}

pub fn enum_description<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
) -> Result<Option<EnumDescription<'tcx>>, String> {
    let ty::Adt(adt, _) = ty.kind() else {
        return Ok(None);
    };
    if !adt.is_enum() {
        return Ok(None);
    }
    let layout = tcx
        .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
        .map_err(|error| format!("no enum layout for `{ty}`: {error:?}"))?;
    let discriminants = adt.discriminants(tcx).map(|(_, discr)| discr.val).collect();
    let (tag, encoding) = match &layout.variants {
        Variants::Empty => (None, EnumEncoding::Empty),
        Variants::Single { index } => (
            None,
            EnumEncoding::Single {
                variant: index.as_u32(),
            },
        ),
        Variants::Multiple {
            tag,
            tag_encoding,
            tag_field,
            ..
        } => {
            let tag_ty = match tag.primitive() {
                Primitive::Int(integer, signed) => integer_type(tcx, integer, signed),
                Primitive::Pointer(_) => tcx.types.usize,
                Primitive::Float(_) => {
                    return Err(format!("floating point enum tag in `{ty}`"));
                }
            };
            let tag = Some(TagDescription {
                ty: tag_ty,
                offset: layout.fields.offset(tag_field.as_usize()).bytes(),
            });
            let encoding = match tag_encoding {
                TagEncoding::Direct => EnumEncoding::Direct,
                TagEncoding::Niche {
                    untagged_variant,
                    niche_variants,
                    niche_start,
                } => EnumEncoding::Niche {
                    untagged_variant: untagged_variant.as_u32(),
                    niche_variants: niche_variants.start().as_u32()..=niche_variants.end().as_u32(),
                    niche_start: *niche_start,
                },
            };
            (tag, encoding)
        }
    };
    Ok(Some(EnumDescription {
        tag,
        encoding,
        discriminants,
    }))
}

fn integer_type<'tcx>(tcx: TyCtxt<'tcx>, integer: Integer, signed: bool) -> Ty<'tcx> {
    match (integer, signed) {
        (Integer::I8, false) => tcx.types.u8,
        (Integer::I16, false) => tcx.types.u16,
        (Integer::I32, false) => tcx.types.u32,
        (Integer::I64, false) => tcx.types.u64,
        (Integer::I128, false) => tcx.types.u128,
        (Integer::I8, true) => tcx.types.i8,
        (Integer::I16, true) => tcx.types.i16,
        (Integer::I32, true) => tcx.types.i32,
        (Integer::I64, true) => tcx.types.i64,
        (Integer::I128, true) => tcx.types.i128,
    }
}
