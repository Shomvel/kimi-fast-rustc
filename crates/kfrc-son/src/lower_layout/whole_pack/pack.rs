use super::*;

pub(super) struct Pack {
    pub root: NodeId,
    pub ty: TyId,
    pub region: Option<NodeId>,
    pub lanes: Vec<NodeId>,
    pub owned: Vec<NodeId>,
}

pub(super) fn unsigned(graph: &FunctionGraph, ty: TyId) -> Option<u8> {
    let TypeShape::Int {
        width,
        signed: false,
    } = *graph.oracle.shape(ty)
    else {
        return None;
    };
    let bits = width.bits(64);
    (matches!(bits, 8 | 16 | 32 | 64)
        && graph.oracle.size_align(ty)?.size == u64::from(bits / 8)
        && graph.oracle.scalar_validity(ty).is_none()
        && !graph.oracle.semantic().is_union(ty)
        && !graph.oracle.semantic().is_vector(ty))
    .then_some(bits)
}

fn pure(graph: &FunctionGraph, node: NodeId, op: Opcode, ty: TyId) -> bool {
    let inputs = if op == Opcode::IntCast { 1 } else { 2 };
    graph.op(node) == op
        && graph.inputs(node).len() == inputs
        && graph.value_type(node) == &ValueType::Rust(ty)
        && graph.payload(node) == 0
        && graph.control(node).is_none()
}

pub(super) fn plan(
    graph: &FunctionGraph,
    uses: &Uses,
    root: NodeId,
    max_bytes: u64,
    fuel: &mut Fuel,
) -> Option<Pack> {
    let ty = graph.rust_type(root)?;
    let bytes = unsigned(graph, ty)? / 8;
    if !matches!(bytes, 2 | 4 | 8) || u64::from(bytes) > max_bytes {
        return None;
    }
    let mut lanes = vec![NodeId::NONE; usize::from(bytes)];
    let mut owned = HashSet::default();
    let mut pending = vec![root];
    let mut terms = 0;
    while let Some(node) = pending.pop() {
        fuel.take(1)?;
        // A tree has at most 7 ORs, 7 shifts, 8 casts and 8 byte leaves. Shared
        // subexpressions, cycles and duplicated terms all fail this certificate.
        if owned.len() >= 32 || !owned.insert(node) {
            return None;
        }
        if pure(graph, node, Opcode::BitOr, ty) {
            pending.extend_from_slice(graph.inputs(node));
            continue;
        }
        let (cast, offset) = if pure(graph, node, Opcode::ShiftLeft, ty) {
            let count = graph.input(node, 1);
            if graph.op(count) != Opcode::Const || graph.rust_type(count) != Some(ty) {
                return None;
            }
            let bits = graph.constant_value(count);
            if bits == 0 || bits % 8 != 0 || bits >= u128::from(bytes) * 8 {
                return None;
            }
            let cast = graph.input(node, 0);
            if !owned.insert(cast) {
                return None;
            }
            (cast, usize::try_from(bits / 8).ok()?)
        } else {
            (node, 0)
        };
        if !pure(graph, cast, Opcode::IntCast, ty) {
            return None;
        }
        let lane = graph.input(cast, 0);
        let accepted = match graph.op(lane) {
            Opcode::Phi => graph.inputs(lane).len() == 3 && graph.payload(lane) == 0,
            Opcode::Projection => {
                graph.inputs(lane).len() == 1
                    && graph.payload(lane) == 2
                    && graph.control(lane).is_none()
            }
            _ => false,
        };
        if !accepted
            || unsigned(graph, graph.rust_type(lane)?)? != 8
            || !index::only(uses, lane, cast, UseKind::Operand(0))
            || !lanes[offset].is_none()
            || !owned.insert(lane)
        {
            return None;
        }
        lanes[offset] = lane;
        terms += 1;
    }
    if terms != bytes || lanes.iter().any(|lane| lane.is_none()) {
        return None;
    }
    let region = match graph.op(lanes[0]) {
        Opcode::Phi => {
            let region = graph.input(lanes[0], 0);
            if graph.op(region) != Opcode::Region
                || lanes.iter().any(|&lane| {
                    graph.op(lane) != Opcode::Phi
                        || graph.input(lane, 0) != region
                        || (!graph.control(lane).is_none() && graph.control(lane) != region)
                })
            {
                return None;
            }
            Some(region)
        }
        Opcode::Projection
            if lanes
                .iter()
                .all(|&lane| graph.op(lane) == Opcode::Projection) =>
        {
            None
        }
        _ => return None,
    };
    for &node in &owned {
        if node == root {
            continue;
        }
        for usage in uses.of(node) {
            fuel.take(1)?;
            if usage.kind() == UseKind::Control || !owned.contains(&usage.user) {
                return None;
            }
        }
    }
    Some(Pack {
        root,
        ty,
        region,
        lanes,
        owned: owned.into_iter().collect(),
    })
}
