//! Byte-offset lanes shared by the payload scalars of different enum variants.
//!
//! A direct-tag enum's payload bytes never overlap its tag. Payload scalars of
//! different variants with the same byte offset and leaf type use one SSA lane,
//! like an LLVM SROA slice. Inactive payload bytes are unobservable, so a lane
//! may carry another variant's bits; every field read stays behind the S
//! variant guard of its consumer. Lanes never overlap one another, which makes
//! unconditional lane reads and writes order-independent.
use super::*;
use kfrc_body::TagEncoding;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct LaneKey {
    pub offset: u64,
    pub ty: TyId,
}

/// Which (variant, field) payloads of one enum type live in shared lanes.
#[derive(Debug)]
pub(super) struct LanePlan {
    shared: Vec<Vec<bool>>,
}

impl LanePlan {
    pub(super) fn shares(&self, variant: u32, field: usize) -> bool {
        self.shared
            .get(variant as usize)
            .and_then(|fields| fields.get(field))
            .copied()
            .unwrap_or(false)
    }
}

/// Producer of a lane node the first time a lane is demanded.
pub(super) enum LaneSource<'a> {
    /// Read the lane bytes on the enum's fixed effect/control edge.
    Load {
        address: NodeId,
        control: &'a mut NodeId,
        effect: &'a mut NodeId,
    },
    Phi {
        region: NodeId,
        arity: usize,
    },
    Undefined,
}

fn byte_size(graph: &FunctionGraph, ty: TyId) -> Option<u64> {
    graph.oracle.size_align(ty).map(|layout| layout.size)
}

/// Scalar leaves of a field type if every byte it represents is a lane leaf.
fn flat_leaves(graph: &FunctionGraph, ty: TyId, offset: u64, leaves: &mut Vec<LaneKey>) -> bool {
    let semantic = graph.oracle.semantic();
    let Some(size) = byte_size(graph, ty) else {
        return false;
    };
    if semantic.maybe_uninit_inner(ty).is_some() || semantic.is_union(ty) || semantic.is_vector(ty)
    {
        return false;
    }
    if semantic.is_enum(ty) {
        // A fieldless direct enum is exactly its physical tag scalar.
        let Some(layout) = graph.oracle.enums.get(&ty) else {
            return false;
        };
        let (TagEncoding::Direct { .. }, Some(tag)) = (&layout.encoding, layout.tag) else {
            return false;
        };
        let fieldless = graph
            .oracle
            .aggregates
            .get(&ty)
            .is_some_and(|aggregate| aggregate.variants.iter().all(Vec::is_empty));
        if !fieldless || !matches!(byte_size(graph, tag.ty), Some(1 | 2 | 4 | 8)) {
            return false;
        }
        leaves.push(LaneKey {
            offset: offset + tag.offset,
            ty: tag.ty,
        });
        return true;
    }
    if matches!(graph.oracle.shape(ty), TypeShape::Array { .. }) {
        return false;
    }
    if semantic.is_product(ty) && !matches!(graph.oracle.shape(ty), TypeShape::Unit) {
        let Some(aggregate) = graph.oracle.aggregates.get(&ty) else {
            return false;
        };
        return aggregate
            .fields
            .iter()
            .all(|field| flat_leaves(graph, field.ty, offset + field.offset, leaves));
    }
    match size {
        0 => true,
        1 | 2 | 4 | 8 => {
            leaves.push(LaneKey { offset, ty });
            true
        }
        _ => false,
    }
}

pub(super) fn plan(graph: &FunctionGraph, ty: TyId) -> Option<LanePlan> {
    let layout = graph.oracle.enums.get(&ty)?;
    let TagEncoding::Direct { .. } = layout.encoding else {
        return None;
    };
    let tag = layout.tag?;
    let tag_end = tag.offset + byte_size(graph, tag.ty)?;
    let variants = &graph.oracle.aggregates.get(&ty)?.variants;
    if graph.oracle.semantic().variant_count(ty)? != variants.len() {
        return None;
    }
    let candidates: Vec<Vec<Option<Vec<LaneKey>>>> = variants
        .iter()
        .map(|fields| {
            fields
                .iter()
                .map(|field| {
                    let mut leaves = Vec::new();
                    flat_leaves(graph, field.ty, field.offset, &mut leaves).then_some(leaves)
                })
                .collect()
        })
        .collect();
    let mut ranges: Vec<(u64, u64, LaneKey)> = Vec::new();
    for leaves in candidates.iter().flatten().flatten() {
        for &key in leaves {
            ranges.push((key.offset, key.offset + byte_size(graph, key.ty)?, key));
        }
    }
    ranges.sort();
    ranges.dedup();
    // A key overlapping the tag or a differently keyed range keeps the
    // per-variant representation for every field that contains it.
    let conflicting: HashSet<LaneKey> = ranges
        .iter()
        .filter(|&&(start, end, key)| {
            (start < tag_end && tag.offset < end)
                || ranges.iter().any(|&(other_start, other_end, other)| {
                    other != key && start < other_end && other_start < end
                })
        })
        .map(|&(_, _, key)| key)
        .collect();
    let shared: Vec<Vec<bool>> = candidates
        .iter()
        .map(|fields| {
            fields
                .iter()
                .map(|leaves| {
                    leaves
                        .as_ref()
                        .is_some_and(|leaves| leaves.iter().all(|key| !conflicting.contains(key)))
                })
                .collect()
        })
        .collect();
    shared
        .iter()
        .flatten()
        .any(|&shared| shared)
        .then_some(LanePlan { shared })
}

impl LayoutLowering<'_> {
    pub(super) fn lane_plan(&mut self, ty: TyId) -> Option<Rc<LanePlan>> {
        if let Some(plan) = self.lane_plans.get(&ty) {
            return plan.clone();
        }
        let plan = plan(self.graph, ty).map(Rc::new);
        self.lane_plans.insert(ty, plan.clone());
        plan
    }

    fn lane_node(
        &mut self,
        key: LaneKey,
        source: &mut LaneSource<'_>,
        lanes: &mut BTreeMap<LaneKey, NodeId>,
    ) -> NodeId {
        if let Some(&node) = lanes.get(&key) {
            return node;
        }
        let node = match source {
            LaneSource::Load {
                address,
                control,
                effect,
            } => {
                let pointer = self.field_address(*address, key.offset, key.ty);
                self.load(pointer, key.ty, control, effect)
            }
            LaneSource::Phi { region, arity } => {
                let phi = self.graph.add_growable_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    *arity,
                    0,
                    ValueType::Rust(key.ty),
                );
                self.graph.set_input(phi, 0, *region);
                self.stats.scalar_phis += 1;
                phi
            }
            LaneSource::Undefined => self.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(key.ty),
            ),
        };
        lanes.insert(key, node);
        node
    }

    /// Typed value of a shared payload field whose leaves are lane nodes.
    pub(super) fn lane_value(
        &mut self,
        ty: TyId,
        offset: u64,
        demand: &Demand,
        source: &mut LaneSource<'_>,
        lanes: &mut BTreeMap<LaneKey, NodeId>,
    ) -> Result<Value, LayoutError> {
        let demand = self.demand_for_type(ty, demand.clone());
        if demand.is_none() {
            return Ok(self.omitted(ty));
        }
        let parts = if self.graph.oracle.semantic().is_enum(ty) {
            let tag = self.graph.oracle.enums[&ty]
                .tag
                .ok_or_else(|| LayoutError::Malformed("lane enum lacks tag".into()))?;
            let node = self.lane_node(
                LaneKey {
                    offset: offset + tag.offset,
                    ty: tag.ty,
                },
                source,
                lanes,
            );
            let count = self.graph.oracle.semantic().variant_count(ty).unwrap_or(0);
            Parts::Variant {
                variant: VariantScalar::Direct(node),
                variants: vec![Vec::new(); count],
            }
        } else if self.composite(ty) {
            let mut values = Vec::new();
            for (index, field) in self.fields(ty, None)?.into_iter().enumerate() {
                values.push(self.lane_value(
                    field.ty,
                    offset + field.offset,
                    &demand.child(DemandKey::Field(index as u32)),
                    source,
                    lanes,
                )?);
            }
            Parts::Fields(values)
        } else if self
            .graph
            .oracle
            .size_align(ty)
            .is_some_and(|layout| layout.size == 0)
        {
            Parts::Scalar(self.graph.add_constant(0, ty))
        } else {
            Parts::Scalar(self.lane_node(LaneKey { offset, ty }, source, lanes))
        };
        Ok(Value { ty, parts })
    }

    /// Payload trees for every variant: shared fields from `source` lanes,
    /// the remaining fields from `unshared`.
    pub(super) fn lane_variants(
        &mut self,
        ty: TyId,
        demand: &Demand,
        plan: &LanePlan,
        lanes: &mut BTreeMap<LaneKey, NodeId>,
        mut unshared: impl FnMut(&mut Self, TyId, &Demand) -> Result<Value, LayoutError>,
        source: &mut LaneSource<'_>,
    ) -> Result<Vec<Vec<Value>>, LayoutError> {
        let count = self
            .graph
            .oracle
            .semantic()
            .variant_count(ty)
            .ok_or_else(|| LayoutError::Malformed("enum missing semantic variants".into()))?;
        let mut variants = Vec::with_capacity(count);
        for index in 0..count as u32 {
            let mut fields = Vec::new();
            for (field_index, field) in self.fields(ty, Some(index))?.into_iter().enumerate() {
                let child = demand.child(DemandKey::VariantField(index, field_index as u32));
                fields.push(if plan.shares(index, field_index) {
                    self.lane_value(field.ty, field.offset, &child, source, lanes)?
                } else {
                    unshared(self, field.ty, &child)?
                });
            }
            variants.push(fields);
        }
        Ok(variants)
    }

    /// Lane leaves of a shared field value, keyed by enum byte offset.
    /// Omitted subtrees have no leaves; a materializing caller rejects them.
    pub(super) fn lane_leaves(
        &mut self,
        value: &Value,
        offset: u64,
        materializing: bool,
        leaves: &mut Vec<(LaneKey, NodeId)>,
    ) -> Result<(), LayoutError> {
        let enum_tag = |this: &Self| -> Result<LaneKey, LayoutError> {
            let tag = this
                .graph
                .oracle
                .enums
                .get(&value.ty)
                .and_then(|layout| layout.tag)
                .ok_or_else(|| LayoutError::Malformed("lane enum lacks tag".into()))?;
            Ok(LaneKey {
                offset: offset + tag.offset,
                ty: tag.ty,
            })
        };
        match &value.parts {
            Parts::Omitted if materializing => {
                return Err(LayoutError::Malformed(
                    "omitted value reached physical materialization".into(),
                ));
            }
            Parts::Omitted => {}
            Parts::Scalar(node) => {
                if self
                    .graph
                    .oracle
                    .size_align(value.ty)
                    .is_some_and(|layout| layout.size != 0)
                {
                    leaves.push((
                        LaneKey {
                            offset,
                            ty: value.ty,
                        },
                        *node,
                    ));
                }
            }
            Parts::Fields(values) => {
                let fields = self.fields(value.ty, None)?;
                if fields.len() != values.len() {
                    return Err(LayoutError::Malformed("lane field count mismatch".into()));
                }
                for (value, field) in values.iter().zip(fields) {
                    self.lane_leaves(value, offset + field.offset, materializing, leaves)?;
                }
            }
            Parts::Variant { variant, .. } | Parts::VariantTag(variant) => {
                leaves.push((enum_tag(self)?, variant.node()));
            }
            Parts::KnownVariant { scalar, .. } => {
                leaves.push((enum_tag(self)?, scalar.node()));
            }
            Parts::EncodedVariant(raw) => leaves.push((enum_tag(self)?, *raw)),
            _ => {
                return Err(LayoutError::Malformed(
                    "shared enum lane holds a non-scalar representation".into(),
                ));
            }
        }
        Ok(())
    }

    /// Record `value` for a lane. Different variants may supply poison for a
    /// lane; any two defined suppliers must be the same SSA node.
    pub(super) fn assign_lane<Lane: Ord>(
        &self,
        lanes: &mut BTreeMap<Lane, NodeId>,
        lane: Lane,
        value: NodeId,
    ) -> Result<(), LayoutError> {
        match lanes.get(&lane) {
            None => {
                lanes.insert(lane, value);
            }
            Some(&existing) if existing == value || self.graph.op(value) == Opcode::Undefined => {}
            Some(&existing) if self.graph.op(existing) == Opcode::Undefined => {
                lanes.insert(lane, value);
            }
            Some(_) => {
                return Err(LayoutError::Malformed(
                    "shared enum lane has disagreeing sources".into(),
                ));
            }
        }
        Ok(())
    }

    /// Fill a lane-shared enum Phi. Each lane Phi receives one input per edge:
    /// the active constructor's leaf, or the lane shared by a variant source.
    pub(super) fn fill_lane_phi(
        &mut self,
        target: &Value,
        source: &Value,
        slot: usize,
        plan: &LanePlan,
    ) -> Result<(), LayoutError> {
        let Parts::Variant {
            variant: target_scalar,
            variants: targets,
        } = &target.parts
        else {
            return Err(LayoutError::Malformed("lane Phi target is not enum".into()));
        };
        let mut incoming: BTreeMap<NodeId, NodeId> = BTreeMap::new();
        match &source.parts {
            Parts::KnownVariant {
                variant: active,
                scalar,
                fields,
            } => {
                self.graph
                    .set_input(target_scalar.node(), slot, scalar.node());
                for (case, case_targets) in targets.iter().enumerate() {
                    let layout = self.fields(target.ty, Some(case as u32))?;
                    if case_targets.len() != layout.len() {
                        return Err(LayoutError::Malformed(
                            "lane Phi variant shape mismatch".into(),
                        ));
                    }
                    if case != *active as usize {
                        continue;
                    }
                    if fields.len() != layout.len() {
                        return Err(LayoutError::Malformed(
                            "known variant Phi field shape mismatch".into(),
                        ));
                    }
                    for (index, ((target_field, source_field), field)) in
                        case_targets.iter().zip(fields).zip(&layout).enumerate()
                    {
                        if plan.shares(case as u32, index) {
                            self.incoming_lanes(
                                target_field,
                                source_field,
                                field.offset,
                                &mut incoming,
                            )?;
                        } else {
                            self.fill_phi(target_field, source_field, slot)?;
                        }
                    }
                }
                // Inactive lanes not written by the active variant are poison.
                for (case, case_targets) in targets.iter().enumerate() {
                    if case == *active as usize {
                        continue;
                    }
                    let layout = self.fields(target.ty, Some(case as u32))?;
                    for (index, (target_field, field)) in
                        case_targets.iter().zip(&layout).enumerate()
                    {
                        if !plan.shares(case as u32, index) {
                            self.fill_phi_poison(target_field, slot)?;
                            continue;
                        }
                        let mut leaves = Vec::new();
                        self.lane_leaves(target_field, field.offset, false, &mut leaves)?;
                        for (key, lane) in leaves {
                            if !incoming.contains_key(&lane) {
                                let poison = self.graph.add_node(
                                    Opcode::Undefined,
                                    NodeId::NONE,
                                    &[],
                                    0,
                                    ValueType::Rust(key.ty),
                                );
                                incoming.insert(lane, poison);
                            }
                        }
                    }
                }
            }
            Parts::Variant {
                variant: source_scalar,
                variants: sources,
            } if sources.len() == targets.len() => {
                self.graph
                    .set_input(target_scalar.node(), slot, source_scalar.node());
                for (case, (case_targets, case_sources)) in targets.iter().zip(sources).enumerate()
                {
                    let layout = self.fields(target.ty, Some(case as u32))?;
                    if case_targets.len() != case_sources.len()
                        || case_targets.len() != layout.len()
                    {
                        return Err(LayoutError::Malformed("variant Phi shape mismatch".into()));
                    }
                    for (index, ((target_field, source_field), field)) in case_targets
                        .iter()
                        .zip(case_sources)
                        .zip(&layout)
                        .enumerate()
                    {
                        if plan.shares(case as u32, index) {
                            self.incoming_lanes(
                                target_field,
                                source_field,
                                field.offset,
                                &mut incoming,
                            )?;
                        } else {
                            self.fill_phi(target_field, source_field, slot)?;
                        }
                    }
                }
            }
            _ => return Err(LayoutError::Malformed("Phi field shape mismatch".into())),
        }
        for (lane, value) in incoming {
            self.graph.set_input(lane, slot, value);
        }
        Ok(())
    }

    fn incoming_lanes(
        &mut self,
        target: &Value,
        source: &Value,
        offset: u64,
        incoming: &mut BTreeMap<NodeId, NodeId>,
    ) -> Result<(), LayoutError> {
        if target.ty != source.ty {
            return Err(LayoutError::Malformed("Phi field type mismatch".into()));
        }
        let mut target_leaves = Vec::new();
        self.lane_leaves(target, offset, false, &mut target_leaves)?;
        if target_leaves.is_empty() {
            return Ok(());
        }
        let mut source_leaves = Vec::new();
        self.lane_leaves(source, offset, false, &mut source_leaves)?;
        let source_leaves: HashMap<LaneKey, NodeId> = source_leaves.into_iter().collect();
        for (key, lane) in target_leaves {
            let value = *source_leaves.get(&key).ok_or_else(|| {
                LayoutError::Malformed("lane source omitted a demanded leaf".into())
            })?;
            self.assign_lane(incoming, lane, value)?;
        }
        Ok(())
    }

    /// Unconditionally store every lane of a dynamic-variant value.
    pub(super) fn encode_lanes(
        &mut self,
        ty: TyId,
        variants: &[Vec<Value>],
        plan: &LanePlan,
        address: NodeId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<(), LayoutError> {
        let mut lanes: BTreeMap<LaneKey, NodeId> = BTreeMap::new();
        for (case, fields) in variants.iter().enumerate() {
            let layout = self.fields(ty, Some(case as u32))?;
            if fields.len() != layout.len() {
                return Err(LayoutError::Malformed(
                    "variant field layout mismatch".into(),
                ));
            }
            for (index, (value, field)) in fields.iter().zip(&layout).enumerate() {
                if !plan.shares(case as u32, index) {
                    continue;
                }
                let mut leaves = Vec::new();
                self.lane_leaves(value, field.offset, true, &mut leaves)?;
                for (key, node) in leaves {
                    self.assign_lane(&mut lanes, key, node)?;
                }
            }
        }
        for (key, value) in lanes {
            let pointer = self.field_address(address, key.offset, key.ty);
            self.store(pointer, value, control, effect);
        }
        Ok(())
    }
}
