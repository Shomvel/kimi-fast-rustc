//! Capture physical tags only when actual execution consumers need those bytes.
use super::*;
use crate::analysis::uses::{UseKind, Uses};
use kfrc_body::TagEncoding;

fn scalar_niche_transport() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("KFRC_SCALAR_NICHE_TRANSPORT").as_deref() != Ok("0"))
}

/// The entire enum is one initialized tag scalar. Its only payload is a
/// fieldless enum occupying those same bytes, so an observed valid value can
/// be copied without decoding and rebuilding its alternatives.
fn scalar_niche_field(graph: &FunctionGraph, ty: TyId) -> Option<TyId> {
    if !scalar_niche_transport() {
        return None;
    }
    let layout = graph.oracle.enums.get(&ty)?;
    let TagEncoding::Niche {
        untagged_variant, ..
    } = layout.encoding
    else {
        return None;
    };
    let tag = layout.tag?;
    let size = graph.oracle.size_align(ty)?.size;
    if tag.offset != 0
        || !matches!(size, 1 | 2 | 4 | 8)
        || graph.oracle.size_align(tag.ty)?.size != size
    {
        return None;
    }
    let variants = &graph.oracle.aggregates.get(&ty)?.variants;
    if variants
        .iter()
        .enumerate()
        .any(|(index, fields)| index != untagged_variant as usize && !fields.is_empty())
    {
        return None;
    }
    let [field] = variants.get(untagged_variant as usize)?.as_slice() else {
        return None;
    };
    if field.offset != 0 || graph.oracle.size_align(field.ty)?.size != size {
        return None;
    }
    let payload = graph.oracle.enums.get(&field.ty)?;
    let payload_tag = payload.tag?;
    if !matches!(payload.encoding, TagEncoding::Direct { .. })
        || payload_tag.offset != 0
        || graph.oracle.size_align(payload_tag.ty)?.size != size
        || !graph
            .oracle
            .aggregates
            .get(&field.ty)?
            .variants
            .iter()
            .all(Vec::is_empty)
    {
        return None;
    }
    Some(field.ty)
}

// A niche payload can reuse the captured tag only when it occupies those exact
// bytes. Other enum fields must still be captured by guarded typed loads.
fn tag_field(graph: &FunctionGraph, ty: TyId, variant: u32, field: u32) -> Option<TyId> {
    let layout = graph.oracle.enums.get(&ty)?;
    let TagEncoding::Niche {
        untagged_variant, ..
    } = layout.encoding
    else {
        return None;
    };
    if variant != untagged_variant || field != 0 {
        return None;
    }
    let tag = layout.tag?;
    let fields = graph
        .oracle
        .aggregates
        .get(&ty)?
        .variants
        .get(variant as usize)?;
    if fields.len() != 1 || fields[0].offset != tag.offset {
        return None;
    }
    let field = fields[0];
    if graph.oracle.size_align(ty)? != graph.oracle.size_align(field.ty)?
        || graph.oracle.size_align(tag.ty)?.size != graph.oracle.size_align(field.ty)?.size
        || (graph.oracle.semantic().thin_pointee(field.ty).is_none()
            && scalar_niche_field(graph, ty) != Some(field.ty))
    {
        return None;
    }
    Some(field.ty)
}

pub(super) fn encoded_demands(graph: &FunctionGraph) -> HashSet<NodeId> {
    fn eligible(graph: &FunctionGraph, uses: &Uses, node: NodeId, ty: TyId) -> bool {
        let consumers = uses.of(node);
        !consumers.is_empty()
            && consumers.iter().all(|use_| {
                if graph.op(use_.user) == Opcode::StoreValue && use_.kind() == UseKind::Operand(2) {
                    return scalar_niche_field(graph, ty).is_some();
                }
                if use_.kind() != UseKind::Operand(0) {
                    return false;
                }
                match graph.op(use_.user) {
                    Opcode::VariantSwitch | Opcode::VariantOf | Opcode::Discriminant => true,
                    // Witnesses are compile-time proof uses, not requests for
                    // a runtime payload or a second copy of the enum value.
                    Opcode::VariantWitness => true,
                    Opcode::VariantField if graph.inputs(use_.user).len() == 2 => {
                        let (variant, field) =
                            graph.variant_fields[graph.payload(use_.user) as usize];
                        let witness = graph.input(use_.user, 1);
                        graph.op(witness) == Opcode::VariantWitness
                            && graph.input(witness, 0) == node
                            && graph.payload(witness) == variant
                            && tag_field(graph, ty, variant, field).is_some()
                    }
                    _ => false,
                }
            })
    }
    let uses = Uses::new(graph);
    graph
        .ids()
        .filter(|&node| {
            graph.op(node) == Opcode::Projection
                && graph.rust_type(node).is_some_and(|ty| {
                    graph.oracle.semantic().is_enum(ty)
                        && graph.oracle.enums.get(&ty).is_some_and(|layout| {
                            layout.tag.is_some()
                                && matches!(
                                    layout.encoding,
                                    TagEncoding::Direct { .. } | TagEncoding::Niche { .. }
                                )
                        })
                        && eligible(graph, &uses, node, ty)
                })
        })
        .collect()
}

impl LayoutLowering<'_> {
    pub(super) fn decode_encoded(
        &mut self,
        address: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<Value, LayoutError> {
        let tag = self.graph.oracle.enums[&ty]
            .tag
            .ok_or_else(|| LayoutError::Malformed("encoded enum lacks tag".into()))?;
        let pointer = self.field_address(address, tag.offset, tag.ty);
        let raw = self.load(pointer, tag.ty, control, effect);
        Ok(Value {
            ty,
            parts: Parts::EncodedVariant(raw),
        })
    }

    pub(super) fn encoded_field(
        &mut self,
        node: NodeId,
        ty: TyId,
        raw: NodeId,
        variant: u32,
        field: u32,
    ) -> Result<Value, LayoutError> {
        let field_ty = tag_field(self.graph, ty, variant, field).ok_or_else(|| {
            LayoutError::Malformed("tag-only enum cannot supply this payload".into())
        })?;
        if self.graph.oracle.semantic().is_enum(field_ty) {
            let tag_ty = self.graph.oracle.enums[&field_ty].tag.unwrap().ty;
            let raw = if self.graph.rust_type(raw) == Some(tag_ty) {
                raw
            } else {
                self.graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[raw],
                    0,
                    ValueType::Rust(tag_ty),
                )
            };
            let count = self
                .graph
                .oracle
                .semantic()
                .variant_count(field_ty)
                .unwrap();
            let value = Value {
                ty: field_ty,
                parts: Parts::Variant {
                    variant: self.variant_scalar(field_ty, raw),
                    variants: vec![Vec::new(); count],
                },
            };
            self.values.insert(node, value.clone());
            self.retired.push(node);
            return Ok(value);
        }
        let scalar = if self.graph.rust_type(raw) == Some(field_ty) {
            raw
        } else {
            self.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[raw],
                0,
                ValueType::Rust(field_ty),
            )
        };
        let value = Value {
            ty: field_ty,
            parts: Parts::Scalar(scalar),
        };
        self.graph.replace(node, scalar);
        self.values.insert(node, value.clone());
        Ok(value)
    }

    pub(super) fn encode_scalar_niche(
        &mut self,
        ty: TyId,
        raw: NodeId,
        address: NodeId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<(), LayoutError> {
        if scalar_niche_field(self.graph, ty).is_none() {
            return Err(LayoutError::Malformed(
                "tag-only enum escaped its discriminant consumers".into(),
            ));
        }
        // Selection proved that every byte belongs to the captured scalar.
        // Keep the load at its original point, even when the destination may
        // alias the source or a subsequent call mutates the borrowed storage.
        self.store(address, raw, control, effect);
        Ok(())
    }
}
