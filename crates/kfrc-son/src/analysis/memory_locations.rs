//! Bounded, per-graph address and access-range queries over the existing IR.
//! No type-based disjointness, integer-address equality or new memory SSA.
use super::*;
use crate::ArgPermission;

/// A non-wrapping byte interval relative to one exact SSA pointer source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryLocation {
    pub root: NodeId,
    pub offset: u64,
    pub size: u64,
    pub align: u64,
    pub ty: TyId,
}

/// Invalidated by graph edits that change addresses, storage or escape uses.
/// Each first address query visits at most 32 address steps, also capped by a
/// function-wide budget. Cached failures stay failures for this snapshot.
pub struct MemoryLocations {
    facts: MemoryFacts,
    locations: HashMap<NodeId, Option<MemoryLocation>>,
    fuel: usize,
    pub node_visits: usize,
    pub budget_bailouts: usize,
}

impl MemoryLocations {
    pub fn new(graph: &FunctionGraph) -> Self {
        let facts = MemoryFacts::new(graph);
        let visits = facts.node_visits;
        let mut locations = Self::from_facts(graph, facts);
        locations.node_visits = visits;
        locations
    }

    /// Reuse this graph snapshot's provenance. Its construction visits belong
    /// to the caller; counters here include only subsequent location queries.
    pub(crate) fn from_facts(graph: &FunctionGraph, facts: MemoryFacts) -> Self {
        Self {
            node_visits: 0,
            facts,
            locations: HashMap::default(),
            fuel: graph.len().saturating_mul(16).clamp(64, 100_000),
            budget_bailouts: 0,
        }
    }

    pub fn location(&mut self, graph: &FunctionGraph, address: NodeId) -> Option<MemoryLocation> {
        let address = graph.resolve(address);
        if let Some(&location) = self.locations.get(&address) {
            return location;
        }
        let location = self.find(graph, address);
        self.locations.insert(address, location);
        location
    }

    fn find(&mut self, graph: &FunctionGraph, address: NodeId) -> Option<MemoryLocation> {
        let ty = graph
            .oracle
            .semantic()
            .thin_pointee(graph.rust_type(address)?)?;
        let layout = graph.oracle.size_align(ty)?;
        let mut root = address;
        let mut offset = 0u64;
        for _ in 0..32 {
            if self.fuel == 0 {
                self.budget_bailouts += 1;
                return None;
            }
            self.fuel -= 1;
            self.node_visits += 1;
            let add = match graph.op(root) {
                Opcode::PointerCast => 0,
                Opcode::FieldAddr => {
                    let base = graph.resolve(graph.input(root, 0));
                    let container = graph
                        .oracle
                        .semantic()
                        .thin_pointee(graph.rust_type(base)?)?;
                    // Variant/union storage overlaps and may depend on a tag.
                    if graph.oracle.semantic().is_enum(container)
                        || graph.oracle.semantic().is_union(container)
                    {
                        return None;
                    }
                    let field = graph.payload(root);
                    match graph.oracle.shape(container) {
                        TypeShape::Array { element, .. } => graph
                            .oracle
                            .size_align(*element)?
                            .size
                            .checked_mul(u64::from(field))?,
                        _ => graph.oracle.field(container, None, field)?.offset,
                    }
                }
                Opcode::IndexAddr => {
                    let index = graph.resolve(graph.input(root, 1));
                    if graph.op(index) != Opcode::Const {
                        return None;
                    }
                    let TypeShape::Int { width, signed } =
                        graph.oracle.shape(graph.rust_type(index)?)
                    else {
                        return None;
                    };
                    let bits = width.bits(64);
                    let raw = graph.constant_value(index);
                    // S normalizes Const to its source width; L converts
                    // indices to usize, sign-extending signed sources. Only
                    // canonical, nonnegative <=64-bit indices agree with
                    // both contracts without a new integer-conversion proof.
                    if bits > 64
                        || raw >= (1u128 << bits)
                        || (*signed && raw & (1u128 << (bits - 1)) != 0)
                    {
                        return None;
                    }
                    let index = raw as u64;
                    let pointee = graph
                        .oracle
                        .semantic()
                        .thin_pointee(graph.rust_type(root)?)?;
                    index.checked_mul(graph.oracle.size_align(pointee)?.size)?
                }
                // Unknown provenance remains an exact SSA source. Phi, loaded
                // pointers and call results are never split into allocations.
                _ => {
                    offset.checked_add(layout.size)?;
                    return Some(MemoryLocation {
                        root,
                        offset,
                        size: layout.size,
                        align: layout.align,
                        ty,
                    });
                }
            };
            // Integer conversion/PtrOffset is deliberately absent: its
            // wrapping and provenance contract differs from these S addresses.
            offset = offset.checked_add(add)?;
            root = graph.resolve(graph.input(root, 0));
        }
        self.budget_bailouts += 1;
        None
    }

    /// True only for separate known allocations or disjoint ranges from the
    /// very same SSA root. Unknown results are MayAlias.
    pub fn disjoint(&mut self, graph: &FunctionGraph, a: NodeId, b: NodeId) -> bool {
        if self.separate_allocations(graph, a, b) {
            return true;
        }
        let (Some(a), Some(b)) = (self.location(graph, a), self.location(graph, b)) else {
            return false;
        };
        a.root == b.root
            && (a
                .offset
                .checked_add(a.size)
                .is_some_and(|end| end <= b.offset)
                || b.offset
                    .checked_add(b.size)
                    .is_some_and(|end| end <= a.offset))
    }

    /// Lifetime operations target the allocation root and affect every field.
    /// Different SSA roots alone do not prove different allocations: duplicate
    /// LocalSlot nodes with the same source-local identity alias each other.
    pub fn separate_allocations(&self, graph: &FunctionGraph, a: NodeId, b: NodeId) -> bool {
        let roots = (
            self.facts.root(graph.resolve(a)),
            self.facts.root(graph.resolve(b)),
        );
        let (Some(a), Some(b)) = roots else {
            return false;
        };
        matches!((self.facts.local_identity(a), self.facts.local_identity(b)),
            (Some(a), Some(b)) if a != b)
    }

    /// The byte version alone is not enough for typed snapshot forwarding:
    /// padding, enum decoding and initialization are tied to the same type.
    pub fn same_typed_place(&mut self, graph: &FunctionGraph, a: NodeId, b: NodeId) -> bool {
        let (Some(a), Some(b)) = (self.location(graph, a), self.location(graph, b)) else {
            return false;
        };
        a == b
    }

    /// Static access validity for DSE. Unknown raw pointers do not qualify,
    /// even if a later write would fail in the same way.
    pub fn valid_store(&mut self, graph: &FunctionGraph, address: NodeId, at: NodeId) -> bool {
        let Some(place) = self.location(graph, address) else {
            return false;
        };
        let Some(root_ty) = graph.rust_type(place.root) else {
            return false;
        };
        let Some(pointee) = graph.oracle.semantic().thin_pointee(root_ty) else {
            return false;
        };
        let Some(layout) = graph.oracle.size_align(pointee) else {
            return false;
        };
        if layout.align < place.align || place.offset % place.align != 0 {
            return false;
        }
        let Some(end) = place.offset.checked_add(place.size) else {
            return false;
        };
        match graph.op(place.root) {
            Opcode::LocalSlot if end <= layout.size => {
                let Some(local) = self.facts.local_identity(place.root) else {
                    return false;
                };
                self.facts.is_unescaped_local(local) && self.live_local(graph, place.root, at)
            }
            Opcode::Projection
                if graph.resolve(graph.input(place.root, 0)) == graph.start
                    && graph.payload(place.root) >= 2 =>
            {
                matches!(
                    graph.arg_permission(graph.payload(place.root) as usize - 2),
                    ArgPermission::Unique { dereferenceable: Some(bytes) } if end <= bytes
                )
            }
            _ => false,
        }
    }

    fn live_local(&mut self, graph: &FunctionGraph, root: NodeId, at: NodeId) -> bool {
        let mut control = graph.resolve(graph.control(at));
        for _ in 0..64 {
            self.node_visits += 1;
            if control == graph.start {
                return true; // Invocation-local storage starts live.
            }
            if graph.op(control) != Opcode::Projection {
                return false; // A join/loop needs a separate lifetime proof.
            }
            let source = graph.resolve(graph.input(control, 0));
            match graph.op(source) {
                Opcode::SlotLive | Opcode::SlotDead => {
                    let address = graph.resolve(graph.input(source, 1));
                    if self.facts.root(address) == Some(root) {
                        return graph.op(source) == Opcode::SlotLive;
                    }
                }
                Opcode::Start => return true,
                _ => {}
            }
            control = graph.resolve(graph.control(source));
            if control.is_none() {
                return false;
            }
        }
        self.budget_bailouts += 1;
        false
    }
}
