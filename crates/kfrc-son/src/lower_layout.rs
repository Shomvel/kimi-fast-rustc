//! Specialized Rust values → explicit layout values in the same graph.
//!
//! Composite values use scalar leaves by default, including loop-carried Phi values.
//! Certified whole-array transport uses private snapshots instead.
//! Whole enum transport can retain private snapshots; observable fields and
//! parameter/call-result fields are captured at their original fixed point.
//! An address never stands in for borrowed mutable storage. RecordedTypes is a bootstrap layout
//! provider; this does not yet implement the final MachineTy/answer-key design.

mod array_transport;
mod boundary_transport;
mod cleanup;
mod demand;
mod encoded;
mod ignored_abi;
mod maybe_uninit;
mod memory;
mod observation;
mod owned_transport;
mod private_memory;
mod projections;
mod repeat;
mod representation;
mod semantic_memory;
mod values;
mod variant_encoding;
mod variant_lanes;
mod variant_scalar;
mod variant_switch;
mod variants;

use demand::{Demand, Key as DemandKey};
use kfrc_body::fx_hash::{HashMap, HashSet};
use std::collections::BTreeMap;

use kfrc_body::{IntegerWidth, SizeAlign, TyId, TypeOracle, TypeShape};

use crate::{FunctionGraph, GraphLevel, GraphState, NodeId, Opcode, ValueType};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    NotSpecialized,
    NotVerified,
    Unsupported(String),
    Malformed(String),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSpecialized => f.write_str("layout lowering requires a Specialized graph"),
            Self::NotVerified => f.write_str("layout lowering requires a verified graph"),
            Self::Unsupported(message) => write!(f, "unsupported in layout lowering: {message}"),
            Self::Malformed(message) => write!(f, "malformed layout input: {message}"),
        }
    }
}

impl std::error::Error for LayoutError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutStats {
    pub scalar_phis: usize,
    /// Whole-value enum Phis represented by private owned snapshots.
    pub owned_transport_phis: usize,
    /// Dense whole arrays captured by a representation copy rather than leaves.
    pub transported_arrays: usize,
    /// Complete call-result fields captured without recursive typed decoding.
    pub transported_boundary_fields: usize,
    pub abi_slots: usize,
    pub field_loads: usize,
    pub field_stores: usize,
    pub repeat_loops: usize,
    /// Nodes visited by the single projection-index seed pass.
    pub projection_indexed: usize,
    pub projection_queries: usize,
    pub projection_cache_misses: usize,
    /// Bucket entries checked, including stale entries discarded lazily.
    pub projection_candidates_checked: usize,
    /// Variant conditions on a decoded niche index answered by its tag.
    pub niche_tag_conditions: usize,
    /// Last cases of exhaustive VariantSwitches selected without a compare.
    pub exhaustive_switch_compares_removed: usize,
}

/// Target-dependent code size policy; this does not change Rust value semantics.
#[derive(Clone, Copy, Debug)]
pub struct LayoutOptions {
    pub max_unrolled_memory_bytes: u64,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            max_unrolled_memory_bytes: 128,
        }
    }
}

/// On failure the input remains unchanged. Success invalidates verification;
/// the caller must verify the resulting Layout graph before machine lowering.
pub fn lower_layout(graph: &mut FunctionGraph) -> Result<LayoutStats, LayoutError> {
    lower_layout_with_options(graph, LayoutOptions::default())
}

pub fn lower_layout_with_options(
    graph: &mut FunctionGraph,
    options: LayoutOptions,
) -> Result<LayoutStats, LayoutError> {
    if graph.level != GraphLevel::Specialized {
        return Err(LayoutError::NotSpecialized);
    }
    if graph.state != GraphState::Verified {
        return Err(LayoutError::NotVerified);
    }
    let mut candidate = graph.clone();
    observation::erase_requirements(&mut candidate);
    ignored_abi::erase_ignored_transport(&mut candidate)?;
    let mut projections = projections::ProjectionIndex::new(&candidate);
    let tag_only_loads = tag_only_loads(&candidate, &mut projections);
    let unreachable_switch_edges = unreachable_switch_edges(&candidate);
    let encoded_values = encoded::encoded_demands(&candidate);
    let demands = demand::analyze(&candidate);
    let owned_transport = owned_transport::select(&candidate, &demands);
    let array_transport = array_transport::select(&candidate);
    let boundary_transport =
        boundary_transport::select(&candidate, &demands, &owned_transport, &encoded_values);
    let mut lowering = LayoutLowering {
        graph: &mut candidate,
        projections,
        values: HashMap::default(),
        demands,
        owned_transport,
        array_transport,
        boundary_transport,
        owned_phis: Vec::new(),
        stats: LayoutStats::default(),
        retired: Vec::new(),
        partition: 0,
        tag_only_loads,
        options,
        encoded_values,
        variant_tags: HashMap::default(),
        unreachable_switch_edges,
        opaque_phis: Vec::new(),
        opaque_phi_regions: HashMap::default(),
        lane_plans: HashMap::default(),
    };
    lowering.run()?;
    lowering.projections.write_stats(&mut lowering.stats);
    let stats = lowering.stats;
    candidate.level = GraphLevel::Layout;
    cleanup::run(&mut candidate)?;
    // Permissions belong to S parameters and source calls (RS5.1 erasure
    // point). L parameters follow physical passing modes instead.
    candidate.arg_permissions.clear();
    candidate.callee_permissions.clear();
    // Generated calls and address materialization change escape/provenance
    // facts. Recompute labels on the completed, compact L graph; S labels
    // cannot license Local accesses after an address has reached a call.
    let facts = crate::analysis::memory::MemoryFacts::new(&candidate);
    crate::transforms::memory::partition::run(&mut candidate, &facts);
    candidate.state = GraphState::Sealed;
    *graph = candidate;
    Ok(stats)
}

#[derive(Clone, Debug)]
struct Value {
    ty: TyId,
    parts: Parts,
}

/// Direct encodings stay in the physical tag domain, including through Phis.
/// Other encodings use logical variant indices; a niche's raw bytes may also
/// carry payload, so they cannot replace the index in a general enum value.
#[derive(Clone, Copy, Debug)]
enum VariantScalar {
    Direct(NodeId),
    Index(NodeId),
}

impl VariantScalar {
    fn node(self) -> NodeId {
        match self {
            Self::Direct(node) | Self::Index(node) => node,
        }
    }
}

#[derive(Clone, Debug)]
enum Parts {
    /// No execution consumer requested this subtree. Never an IR Undefined.
    Omitted,
    OpaqueStorage(NodeId),
    /// Private snapshot of a typed value, used only for whole-value transport.
    OwnedStorage(NodeId),
    Scalar(NodeId),
    Fields(Vec<Value>),
    // Nonempty array; repeated elements are immutable SSA values, never aliases
    // to mutable storage. Empty arrays use Fields([]) without an element value.
    Repeat {
        element: Box<Value>,
        count: u64,
        overrides: BTreeMap<u32, Value>,
    },
    Variant {
        variant: VariantScalar,
        variants: Vec<Vec<Value>>,
    },
    /// A constructor owns only its active payload. Inactive fields become
    /// poison on demand; they never allocate a recursively expanded tree.
    KnownVariant {
        variant: u32,
        scalar: VariantScalar,
        fields: Vec<Value>,
    },
    // Only created for loads whose original users all demand the tag. Keeping
    // this distinct prevents an incomplete enum from being materialized.
    VariantTag(VariantScalar),
    // A captured physical tag, used only by semantic tag queries and proven
    // refinements of payload fields represented by those same bytes.
    EncodedVariant(NodeId),
}

struct LayoutLowering<'a> {
    graph: &'a mut FunctionGraph,
    projections: projections::ProjectionIndex,
    values: HashMap<NodeId, Value>,
    demands: HashMap<NodeId, Demand>,
    owned_transport: HashSet<NodeId>,
    array_transport: HashSet<TyId>,
    boundary_transport: boundary_transport::Certificates,
    owned_phis: Vec<owned_transport::OwnedPhiInit>,
    stats: LayoutStats,
    retired: Vec<NodeId>,
    partition: u32,
    tag_only_loads: HashSet<NodeId>,
    options: LayoutOptions,
    encoded_values: HashSet<NodeId>,
    variant_tags: HashMap<NodeId, NodeId>,
    /// VariantSwitch edges whose only successor is `Unreachable`, with that node.
    unreachable_switch_edges: HashMap<NodeId, NodeId>,
    opaque_phis: Vec<maybe_uninit::OpaquePhiCopy>,
    opaque_phi_regions: HashMap<NodeId, NodeId>,
    lane_plans: HashMap<TyId, Option<std::rc::Rc<variant_lanes::LanePlan>>>,
}

/// A VariantSwitch edge that leads only to `Unreachable` is never taken by a
/// valid enum value: an exhaustive match's remaining tags are all covered.
fn unreachable_switch_edges(graph: &FunctionGraph) -> HashMap<NodeId, NodeId> {
    use crate::analysis::uses::{UseKind, Uses};

    let uses = Uses::new(graph);
    graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::Unreachable)
        .filter_map(|unreachable| {
            let edge = graph.resolve(graph.control(unreachable));
            if edge.is_none() || graph.op(edge) != Opcode::Projection {
                return None;
            }
            let source = graph.resolve(graph.input(edge, 0));
            let only_successor = matches!(
                uses.of(edge),
                [only] if only.user == unreachable && only.kind == UseKind::Control
            );
            (graph.op(source) == Opcode::VariantSwitch && only_successor)
                .then_some((edge, unreachable))
        })
        .collect()
}

fn tag_only_loads(
    graph: &FunctionGraph,
    projections: &mut projections::ProjectionIndex,
) -> HashSet<NodeId> {
    use crate::analysis::uses::{UseKind, Uses};

    let uses = Uses::new(graph);
    graph
        .ids()
        .filter(|&node| {
            if graph.op(node) != Opcode::LoadValue {
                return false;
            }
            let Some(result) = projections.find(graph, node, 2) else {
                return false;
            };
            let Some(ty) = graph.rust_type(result) else {
                return false;
            };
            let consumers = uses.of(result);
            graph.oracle.semantic().is_enum(ty)
                && !consumers.is_empty()
                && consumers.iter().all(|use_| {
                    use_.kind == UseKind::Operand(0)
                        && matches!(
                            graph.op(use_.user),
                            Opcode::VariantOf | Opcode::Discriminant
                        )
                })
        })
        .collect()
}

impl LayoutLowering<'_> {
    fn run(&mut self) -> Result<(), LayoutError> {
        let original: Vec<_> = self.graph.ids().collect();
        // Rewrite the ABI result schema even when its projection is unused.
        for node in &original {
            if matches!(
                self.graph.op(*node),
                Opcode::Start | Opcode::Call | Opcode::CallIndirect | Opcode::Opaque
            ) {
                let ty = self.physical_type(self.graph.value_type(*node));
                self.graph.set_value_type(*node, ty);
            }
        }
        // Snapshot aggregate parameters before any program effects. Each
        // subsequent parameter starts after the previous one's field loads.
        for node in &original {
            if self.graph.op(*node) == Opcode::Projection
                && self.graph.input(*node, 0) == self.graph.start
                && self.graph.payload(*node) >= 2
                && let Some(ty) = self.graph.rust_type(*node)
                && self.composite(ty)
            {
                self.expand(*node)?;
            }
        }
        // Loads capture typed values on their fixed effect/control edge.
        // Scalar projections may feed later structured addresses.
        for node in &original {
            match self.graph.op(*node) {
                Opcode::LoadValue => self.lower_load_value(*node)?,
                Opcode::TransmuteValue => self.lower_transmute(*node)?,
                _ => {}
            }
        }
        // Resolve semantic projections before replacing their aggregate roots.
        for node in &original {
            if self.graph.op(*node) != Opcode::VariantWitness
                && (self.graph.op(*node).is_semantic_value()
                    || matches!(
                        self.graph.op(*node),
                        Opcode::LocalSlot
                            | Opcode::FieldAddr
                            | Opcode::VariantFieldAddr
                            | Opcode::IndexAddr
                            | Opcode::PointerCast
                    ))
            {
                let value = self.expand(*node)?;
                if let Parts::Scalar(scalar) = value.parts
                    && scalar != *node
                {
                    self.graph.replace(*node, scalar);
                }
            }
        }
        for node in &original {
            if self.graph.op(*node) == Opcode::VariantSwitch {
                self.lower_variant_switch(*node)?;
            }
        }
        for node in &original {
            let indices: Vec<usize> = match self.graph.op(*node) {
                Opcode::Call => (1..self.graph.inputs(*node).len()).collect(),
                Opcode::CallIndirect => (2..self.graph.inputs(*node).len()).collect(),
                Opcode::Return | Opcode::Opaque => (1..self.graph.inputs(*node).len()).collect(),
                _ => continue,
            };
            for index in indices {
                let argument = self.graph.input(*node, index);
                let Some(ty) = self.graph.rust_type(argument) else {
                    continue;
                };
                if self.composite(ty) {
                    let value = self.expand(argument)?;
                    let slot = self.materialize_before(&value, *node)?;
                    self.graph.set_input(*node, index, slot);
                }
            }
            // A returned aggregate remains a physical ABI object even when its
            // caller discards it. Live projections are decoded on demand.
            if matches!(
                self.graph.op(*node),
                Opcode::Call | Opcode::CallIndirect | Opcode::Opaque
            ) {
                if let Some(result) = self.projections.find(self.graph, *node, 2) {
                    if let Some(ty) = self.graph.rust_type(result) {
                        if self.composite(ty) {
                            self.expand(result)?;
                        }
                    }
                }
            }
        }
        for node in &original {
            if self.graph.op(*node).is_semantic_memory() {
                self.lower_memory_effect(*node)?;
            }
        }
        self.finish_snapshot_phis()?;
        self.graph.resolve_edges();
        for node in self.retired.clone() {
            self.graph.remove_node(node);
        }
        for node in original {
            if self.graph.op(node).is_semantic_value()
                || self.graph.op(node) == Opcode::VariantWitness
                || (matches!(self.graph.op(node), Opcode::Phi | Opcode::Undefined)
                    && self
                        .graph
                        .rust_type(node)
                        .is_some_and(|ty| self.composite(ty)))
            {
                self.graph.remove_node(node);
            }
        }
        self.graph.compact();
        for node in self.graph.ids() {
            if self.graph.op(node).is_semantic_value()
                || self.graph.op(node).is_semantic_memory()
                || matches!(
                    self.graph.op(node),
                    Opcode::VariantSwitch | Opcode::VariantWitness | Opcode::Assume
                )
            {
                return Err(LayoutError::Malformed(format!(
                    "semantic node {node} survived lowering"
                )));
            }
            if let ValueType::Rust(ty) = self.graph.value_type(node) {
                if self.composite(*ty) {
                    return Err(LayoutError::Unsupported(format!(
                        "unlowered aggregate {} at {node}",
                        self.graph.op(node).name()
                    )));
                }
            }
        }
        Ok(())
    }

    fn composite(&self, ty: TyId) -> bool {
        self.graph
            .oracle
            .semantic()
            .maybe_uninit_inner(ty)
            .is_some()
            || self.graph.oracle.semantic().is_enum(ty)
            || self.graph.oracle.semantic().is_product(ty)
                && !matches!(self.graph.oracle.shape(ty), TypeShape::Unit)
    }

    fn physical_type(&self, ty: &ValueType) -> ValueType {
        match ty {
            ValueType::Rust(ty) if self.composite(*ty) => ValueType::LayoutValue(*ty),
            ValueType::Tuple(elements) => {
                ValueType::Tuple(elements.iter().map(|ty| self.physical_type(ty)).collect())
            }
            other => other.clone(),
        }
    }

    fn pointer_type(&mut self, pointee: TyId) -> TyId {
        self.graph.oracle.intern(
            TypeShape::PlaceAddress { pointee },
            Some(SizeAlign { size: 8, align: 8 }),
        )
    }

    fn usize_type(&mut self) -> TyId {
        self.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        )
    }

    fn projection(&mut self, source: NodeId, index: u32, ty: ValueType) -> NodeId {
        if let Some(node) = self.projections.find(self.graph, source, index) {
            return node;
        }
        let node = self
            .graph
            .add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty);
        self.projections.register(self.graph, node);
        node
    }
}
