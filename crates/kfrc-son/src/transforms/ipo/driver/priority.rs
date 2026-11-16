//! Prefer constant-argument scalar calls that can prune control before copying
//! address-only helpers from cold arms. This analysis never rewrites the IR.
use super::Arguments;
use crate::{FunctionGraph, NodeId, Opcode};
use kfrc_body::fx_hash::HashSet;
use kfrc_body::{TypeOracle, TypeShape};

const MAX_PATH: usize = 4;

#[derive(Default)]
pub(super) struct Work {
    pub calls: HashSet<NodeId>,
    pub visits: usize,
    pub bailed_out: bool,
}

pub(super) fn branch_drivers(graph: &FunctionGraph, max_visits: usize) -> Work {
    let mut work = Work::default();
    if max_visits == 0 {
        return work;
    }
    let mut remaining = max_visits;
    let mut checked = HashSet::default();
    for node in graph.ids() {
        if !charge(&mut remaining, &mut work) {
            break;
        }
        if graph.op(node) != Opcode::If {
            continue;
        }
        // S VariantSwitch consumes an enum, not a scalar. No value-memory,
        // Phi, call, or aggregate edge is followed by this scalar-only query.
        let mut pending = vec![(graph.input(node, 0), 0)];
        let mut seen = HashSet::default();
        while let Some((value, depth)) = pending.pop() {
            let value = graph.resolve(value);
            if depth > MAX_PATH || !seen.insert((value, depth)) {
                continue;
            }
            if !charge(&mut remaining, &mut work) {
                break;
            }
            if !graph.control(value).is_none() {
                continue;
            }
            match graph.op(value) {
                Opcode::Projection if graph.payload(value) >= 2 => {
                    let call = graph.input(value, 0);
                    if graph.op(call) != Opcode::Call
                        || !checked.insert(call)
                        || !graph.rust_type(value).is_some_and(|ty| {
                            matches!(
                                graph.oracle.shape(ty),
                                TypeShape::Int { .. } | TypeShape::Bool | TypeShape::Char
                            )
                        })
                    {
                        continue;
                    }
                    let (constant, visits) = Arguments::has_scalar_fact(graph, call, remaining);
                    remaining -= visits;
                    work.visits += visits;
                    if constant {
                        work.calls.insert(call);
                    }
                    if remaining == 0 {
                        work.bailed_out = true;
                        break;
                    }
                }
                Opcode::Compare | Opcode::IntCast | Opcode::BitNot => {
                    pending.extend(graph.inputs(value).iter().map(|&v| (v, depth + 1)));
                }
                _ => {}
            }
        }
        if work.bailed_out {
            break;
        }
    }
    // Partial classification must not give an accidental advantage to the
    // first visited branch. Exhaustion restores the original size ordering.
    if work.bailed_out {
        work.calls.clear();
    }
    work
}

fn charge(remaining: &mut usize, work: &mut Work) -> bool {
    if *remaining == 0 {
        work.bailed_out = true;
        false
    } else {
        *remaining -= 1;
        work.visits += 1;
        true
    }
}
