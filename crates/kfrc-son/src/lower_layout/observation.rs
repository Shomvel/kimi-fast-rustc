//! Publishing S→L discharges source-domain requirements, not runtime checks.
use super::*;

/// S verification has already checked the requirement's contract and position.
/// Publishing is a refinement: source-UB runs need not retain their diagnostic.
/// Erase before representation analysis so proof-only values never build trees.
pub(super) fn erase_requirements(graph: &mut FunctionGraph) {
    let mut projections = projections::ProjectionIndex::new(graph);
    let requirements: Vec<_> = graph
        .ids()
        .filter(|&node| matches!(graph.op(node), Opcode::RequireDefined | Opcode::Assume))
        .collect();
    for node in requirements {
        let control = graph.control(node);
        let effect = graph.input(node, 0);
        if let Some(projection) = projections.find(graph, node, 0) {
            graph.replace(projection, control);
        }
        if let Some(projection) = projections.find(graph, node, 1) {
            graph.replace(projection, effect);
        }
        graph.remove_node(node);
    }
    // Witness proof edges are remapped through the removed continuation but
    // their source evidence was checked before this refinement boundary. Do
    // not reverify the temporary S graph after erasing an Assume's evidence.
    // Fixed loads, calls, trapping arithmetic and their original ordering stay
    // roots. Only the now-unreferenced pure proof cone is removed.
    graph.compact();
}
