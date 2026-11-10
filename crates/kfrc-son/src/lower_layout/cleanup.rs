//! Bounded cleanup of representation-generated scalar code. This runs only
//! after publishing L: undefined source executions may be refined, whereas
//! fixed loads, calls, and trapping operations remain on their control paths.
use super::*;
use crate::ComparePredicate as P;
use crate::transforms::{
    control::{conditions, simplify},
    scalar::instcombine,
};

#[path = "address_cse.rs"]
mod address_cse;

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;

fn bit_conditions(graph: &mut FunctionGraph) {
    let replacements: Vec<_> = graph
        .ids()
        .filter_map(|branch| {
            if graph.op(branch) != Opcode::If {
                return None;
            }
            let condition = graph.input(branch, 0);
            let ty = graph.rust_type(condition)?;
            if !matches!(graph.oracle.shape(ty), TypeShape::Bool) {
                return None;
            }
            // Only the branch observation is refined here, after L has been
            // published. Other users still observe the original value, and
            // fixed producers remain on their original control/effect path.
            if graph.op(condition) != Opcode::Const {
                let (min, max) =
                    crate::analysis::known_bits::of(graph, condition)?.unsigned_bounds();
                if min == max {
                    return Some((branch, ty, min != 0));
                }
            }
            if graph.op(condition) != Opcode::Compare {
                return None;
            }
            let a = crate::analysis::known_bits::of(graph, graph.input(condition, 0))?
                .unsigned_bounds();
            let b = crate::analysis::known_bits::of(graph, graph.input(condition, 1))?
                .unsigned_bounds();
            let answer = match P::from_payload(graph.payload(condition)) {
                P::UnsignedLess if a.1 < b.0 => true,
                P::UnsignedLess if a.0 >= b.1 => false,
                P::UnsignedLessEqual if a.1 <= b.0 => true,
                P::UnsignedLessEqual if a.0 > b.1 => false,
                P::UnsignedGreater if a.0 > b.1 => true,
                P::UnsignedGreater if a.1 <= b.0 => false,
                P::UnsignedGreaterEqual if a.0 >= b.1 => true,
                P::UnsignedGreaterEqual if a.1 < b.0 => false,
                P::Equal if a.1 < b.0 || b.1 < a.0 => false,
                P::NotEqual if a.1 < b.0 || b.1 < a.0 => true,
                _ => return None,
            };
            Some((branch, ty, answer))
        })
        .collect();
    for (branch, ty, answer) in replacements {
        let value = graph.add_constant(u128::from(answer), ty);
        graph.set_input(branch, 0, value);
    }
}

pub(super) fn run(graph: &mut FunctionGraph) -> Result<(), LayoutError> {
    debug_assert_eq!(graph.level, GraphLevel::Layout);
    address_cse::run(graph);
    for _ in 0..2 {
        private_memory::run(graph);
        instcombine::run(graph);
        graph.resolve_edges();
        bit_conditions(graph);
        conditions::run(graph).map_err(|e| LayoutError::Malformed(e.to_string()))?;
        let stats = simplify::run(graph).map_err(|e| LayoutError::Malformed(e.to_string()))?;
        graph.compact();
        if stats.branches_folded == 0 && stats.predecessor_edges_removed == 0 {
            break;
        }
    }
    // SimplifyCFG can introduce boolean inversions when collapsing diamonds.
    // Fold those before publishing the final scalar graph.
    instcombine::run(graph);
    graph.resolve_edges();
    address_cse::run(graph);
    graph.compact();
    Ok(())
}
