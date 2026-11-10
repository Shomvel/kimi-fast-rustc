//! Reuse the unsigned 64-bit result computed while checking overflow.
//!
//! A machine product already leaves its low half in RAX. Publish that value as
//! a second SSA output rather than multiplying again on the success edge.
//! Likewise, the sum used by `sum < lhs` can feed a dominated success path.
//! Overflow remains an ordinary Boolean SSA value: no flags cross instructions.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    op: Opcode,
    lhs: NodeId,
    rhs: NodeId,
    ty: TyId,
}

fn key(graph: &FunctionGraph, node: NodeId) -> Option<Key> {
    let op = match graph.op(node) {
        Opcode::Add => Opcode::Add,
        Opcode::Mul | Opcode::MulOverflow => Opcode::Mul,
        _ => return None,
    };
    // These contracts constrain defined source executions but do not change
    // the low 64 result bits. Reusing a wrapping result is legal at L, including
    // on a guarded success edge whose operation acquired NUW/NSW.
    if graph.payload(node) & !(ArithmeticFlags::NUW | ArithmeticFlags::NSW) != 0 {
        return None;
    }
    let mut lhs = graph.resolve(graph.input(node, 0));
    let mut rhs = graph.resolve(graph.input(node, 1));
    let ty = graph.rust_type(lhs)?;
    if graph.rust_type(rhs) != Some(ty)
        || !matches!(graph.oracle.shape(ty), TypeShape::Int { width, signed: false } if width.bits(64) == 64)
    {
        return None;
    }
    if graph.op(node) != Opcode::MulOverflow && graph.rust_type(node) != Some(ty) {
        return None;
    }
    if lhs.index() > rhs.index() {
        std::mem::swap(&mut lhs, &mut rhs);
    }
    Some(Key { op, lhs, rhs, ty })
}

pub(super) struct Values {
    candidates: HashSet<Key>,
    products: HashSet<Key>,
    overflow: HashMap<(Key, usize), NodeId>,
    available: HashMap<Key, Vec<(usize, VirtualValue)>>,
}

impl Values {
    pub(super) fn new(graph: &FunctionGraph, schedule: &Schedule) -> Self {
        let mut counts = HashMap::<Key, usize>::default();
        let mut products = HashSet::default();
        for node in graph.ids() {
            if let Some(key) = key(graph, node) {
                *counts.entry(key).or_default() += 1;
                if graph.op(node) == Opcode::Mul {
                    products.insert(key);
                }
            }
        }
        let candidates = counts
            .into_iter()
            .filter_map(|(key, count)| (count > 1).then_some(key))
            .collect();
        let mut overflow = HashMap::default();
        for (block, nodes) in schedule.placed.iter().enumerate() {
            for &node in nodes {
                if graph.op(node) == Opcode::MulOverflow
                    && let Some(key) = key(graph, node)
                {
                    overflow.entry((key, block)).or_insert(node);
                }
            }
        }
        Self {
            candidates,
            products,
            overflow,
            available: HashMap::default(),
        }
    }

    pub(super) fn overflow(
        &self,
        graph: &FunctionGraph,
        block: usize,
        node: NodeId,
    ) -> Option<NodeId> {
        if graph.op(node) != Opcode::Mul {
            return None;
        }
        self.overflow.get(&(key(graph, node)?, block)).copied()
    }

    pub(super) fn get(
        &self,
        graph: &FunctionGraph,
        schedule: &Schedule,
        block: usize,
        node: NodeId,
    ) -> Option<VirtualValue> {
        if graph.op(node) == Opcode::MulOverflow {
            return None;
        }
        self.available
            .get(&key(graph, node)?)?
            .iter()
            .rev()
            .find_map(|&(definition, value)| schedule.dominates(definition, block).then_some(value))
    }

    pub(super) fn needs_product(&self, graph: &FunctionGraph, node: NodeId) -> bool {
        key(graph, node).is_some_and(|key| self.products.contains(&key))
    }

    pub(super) fn record(
        &mut self,
        graph: &FunctionGraph,
        block: usize,
        node: NodeId,
        value: VirtualValue,
    ) {
        if let Some(key) = key(graph, node).filter(|key| self.candidates.contains(key)) {
            self.available.entry(key).or_default().push((block, value));
        }
    }

    pub(super) fn record_product(
        &mut self,
        graph: &FunctionGraph,
        block: usize,
        node: NodeId,
        value: VirtualValue,
    ) {
        self.record(graph, block, node, value);
    }
}
