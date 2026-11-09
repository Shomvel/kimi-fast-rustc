//! Capture physical tags only when actual execution consumers need those bytes.
use super::*;
use crate::analysis::uses::{UseKind, Uses};
use kfrc_body::TagEncoding;

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
        || graph.oracle.semantic().thin_pointee(field.ty).is_none()
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
                if use_.kind != UseKind::Operand(0) {
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
}
