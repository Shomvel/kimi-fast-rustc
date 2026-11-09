//! Certificates for complete fields transported from direct-call enum results.
//! Separate from leaf demand: demanding every leaf does not prove byte transport.
use super::*;
use crate::analysis::{
    cfg::ControlFlow,
    uses::{UseKind, Uses},
};
use kfrc_body::TagEncoding;
use std::collections::BTreeSet;

pub(super) type Fields = BTreeSet<(u32, u32)>;
pub(super) type Certificates = HashMap<NodeId, Fields>;

fn supported_type(graph: &FunctionGraph, ty: TyId, seen: &mut HashSet<TyId>) -> bool {
    let semantic = graph.oracle.semantic();
    if graph.oracle.size_align(ty).is_none()
        || semantic.maybe_uninit_inner(ty).is_some()
        || semantic.is_union(ty)
    {
        return false;
    }
    if !seen.insert(ty) {
        return true;
    }
    match graph.oracle.shape(ty) {
        TypeShape::Param { .. }
        | TypeShape::Other { .. }
        | TypeShape::Slice { .. }
        | TypeShape::Str => false,
        // Copy the pointer representation, never recurse into its pointee.
        TypeShape::Reference { .. }
        | TypeShape::RawPointer { .. }
        | TypeShape::PlaceAddress { .. } => true,
        TypeShape::Array { element, .. } => supported_type(graph, *element, seen),
        _ if semantic.is_enum(ty) => semantic.variant_count(ty).is_some_and(|count| {
            graph.oracle.enums.contains_key(&ty)
                && (0..count).all(|variant| {
                    semantic
                        .field_types(ty, Some(variant as u32))
                        .is_some_and(|fields| {
                            fields
                                .into_iter()
                                .all(|field| supported_type(graph, field, seen))
                        })
                })
        }),
        TypeShape::Adt { .. } | TypeShape::Tuple(_) => {
            semantic.field_types(ty, None).is_some_and(|fields| {
                fields
                    .into_iter()
                    .all(|field| supported_type(graph, field, seen))
            })
        }
        _ => true,
    }
}

fn constructor_transports(
    graph: &FunctionGraph,
    uses: &Uses,
    constructor: NodeId,
    owned: &HashSet<NodeId>,
) -> bool {
    !uses.of(constructor).is_empty()
        && uses.of(constructor).iter().all(|use_| {
            matches!(
                (graph.op(use_.user), use_.kind),
                (Opcode::Return, UseKind::Operand(1)) | (Opcode::StoreValue, UseKind::Operand(2))
            ) || (matches!(use_.kind, UseKind::Operand(index) if index > 0)
                && owned.contains(&use_.user))
        })
}

fn extractor_transports(
    graph: &FunctionGraph,
    uses: &Uses,
    extractor: NodeId,
    root: NodeId,
    key: (u32, u32),
    demands: &HashMap<NodeId, Demand>,
    owned: &HashSet<NodeId>,
) -> bool {
    let inputs = graph.inputs(extractor);
    if inputs.len() != 2 || demands.get(&extractor) != Some(&Demand::All) {
        return false;
    }
    let witness = graph.resolve(inputs[1]);
    if graph.op(witness) != Opcode::VariantWitness
        || graph.resolve(graph.input(witness, 0)) != root
        || graph.payload(witness) != key.0
    {
        return false;
    }
    !uses.of(extractor).is_empty()
        && uses.of(extractor).iter().all(|use_| {
            let UseKind::Operand(field) = use_.kind else {
                return false;
            };
            let constructor = use_.user;
            if graph.op(constructor) != Opcode::MakeVariant {
                return false;
            }
            let Some(ty) = graph.rust_type(constructor) else {
                return false;
            };
            graph
                .oracle
                .semantic()
                .field_type(ty, Some(graph.payload(constructor)), field as u32)
                == graph.rust_type(extractor)
                && constructor_transports(graph, uses, constructor, owned)
        })
}

pub(super) fn select(
    graph: &FunctionGraph,
    demands: &HashMap<NodeId, Demand>,
    owned: &HashSet<NodeId>,
    encoded: &HashSet<NodeId>,
) -> Certificates {
    let cfg = ControlFlow::new(graph);
    // Whole-function rejection is intentional for this first boundary snapshot
    // representation: no static slot may be overwritten by another iteration.
    if cfg
        .predecessors
        .iter()
        .enumerate()
        .any(|(index, preds)| preds.iter().any(|&pred| pred >= index))
    {
        return HashMap::new();
    }
    let uses = Uses::new(graph);
    let mut selected = HashMap::new();
    for root in graph.ids() {
        if encoded.contains(&root)
            || graph.op(root) != Opcode::Projection
            || graph.payload(root) != 2
            || graph.op(graph.input(root, 0)) != Opcode::Call
        {
            continue;
        }
        let Some(ty) = graph.rust_type(root) else {
            continue;
        };
        if !graph.oracle.semantic().is_enum(ty)
            || !graph.oracle.enums.get(&ty).is_some_and(|layout| {
                layout.tag.is_some()
                    && matches!(
                        layout.encoding,
                        TagEncoding::Direct { .. } | TagEncoding::Niche { .. }
                    )
            })
        {
            continue;
        }
        if uses.of(root).iter().any(|use_| {
            use_.kind != UseKind::Operand(0)
                || !matches!(
                    graph.op(use_.user),
                    Opcode::VariantSwitch
                        | Opcode::VariantOf
                        | Opcode::Discriminant
                        | Opcode::VariantWitness
                        | Opcode::VariantField
                )
        }) {
            continue;
        }
        let mut extractors: BTreeMap<(u32, u32), Vec<NodeId>> = BTreeMap::new();
        for use_ in uses.of(root) {
            if graph.op(use_.user) == Opcode::VariantField {
                extractors
                    .entry(graph.variant_fields[graph.payload(use_.user) as usize])
                    .or_default()
                    .push(use_.user);
            }
        }
        let keys: Fields = extractors
            .into_iter()
            .filter_map(|(key, fields)| {
                let field_ty = graph.oracle.semantic().field_type(ty, Some(key.0), key.1)?;
                let field = graph
                    .oracle
                    .aggregates
                    .get(&ty)?
                    .variants
                    .get(key.0 as usize)?
                    .get(key.1 as usize)?;
                let field_size = graph.oracle.size_align(field_ty)?.size;
                if field.offset.checked_add(field_size)? > graph.oracle.size_align(ty)?.size {
                    return None;
                }
                if !(graph.oracle.semantic().is_enum(field_ty)
                    || graph.oracle.semantic().is_product(field_ty))
                    || graph.oracle.size_align(field_ty)?.size == 0
                    || !supported_type(graph, field_ty, &mut HashSet::new())
                    || demands
                        .get(&root)?
                        .child(DemandKey::VariantField(key.0, key.1))
                        != Demand::All
                    || !fields.into_iter().all(|field| {
                        extractor_transports(graph, &uses, field, root, key, demands, owned)
                    })
                {
                    return None;
                }
                Some(key)
            })
            .collect();
        if !keys.is_empty() {
            selected.insert(root, keys);
        }
    }
    selected
}

impl LayoutLowering<'_> {
    pub(super) fn capture_owned_field(
        &mut self,
        source: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<Value, LayoutError> {
        let destination = self.opaque_slot(ty)?;
        let size = self.graph.oracle.size_align(ty).unwrap().size;
        self.raw_copy(source, destination, size, control, effect);
        self.stats.transported_boundary_fields += 1;
        Ok(Value {
            ty,
            parts: Parts::OwnedStorage(destination),
        })
    }

    pub(super) fn merge_owned_field(
        &mut self,
        field: Value,
        region: NodeId,
        address: NodeId,
    ) -> Value {
        let pointer = self.pointer_type(field.ty);
        // Inactive-path pointer poison is only a Phi input. Every actual use
        // still requires the original S witness for this exact root/variant.
        let inactive = self.graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(pointer),
        );
        let selected = self.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, address, inactive],
            0,
            ValueType::Rust(pointer),
        );
        Value {
            ty: field.ty,
            parts: Parts::OwnedStorage(selected),
        }
    }
}
