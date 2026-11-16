//! Reuse pure layout addresses with identical SSA provenance. No memory is
//! read, no pointer equality is used, and anchored addresses are not moved.
use super::*;

#[derive(PartialEq, Eq, Hash)]
enum Offset {
    Constant(TyId, u128),
    Value(NodeId),
}

#[derive(PartialEq, Eq, Hash)]
struct Address {
    base: NodeId,
    offset: Offset,
    ty: ValueType,
    payload: u32,
}

pub(super) fn run(graph: &mut FunctionGraph) {
    let nodes: Vec<_> = graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::PtrOffset && graph.control(node).is_none())
        .collect();
    let mut addresses = HashMap::default();
    for node in nodes {
        let base = graph.resolve(graph.input(node, 0));
        let offset = graph.resolve(graph.input(node, 1));
        let offset = if graph.op(offset) == Opcode::Const {
            let Some(ty) = graph.rust_type(offset) else {
                continue;
            };
            let value = graph.constant_value(offset);
            if value == 0
                && graph.payload(node) == 0
                && graph.value_type(node) == graph.value_type(base)
            {
                graph.replace(node, base);
                continue;
            }
            Offset::Constant(ty, value)
        } else {
            Offset::Value(offset)
        };
        let address = Address {
            base,
            offset,
            ty: graph.value_type(node).clone(),
            payload: graph.payload(node),
        };
        if let Some(&existing) = addresses.get(&address) {
            graph.replace(node, existing);
        } else {
            addresses.insert(address, node);
        }
    }
    graph.resolve_edges();
}
