//! Remove redundant state Phis before asking scalar cleanup about the lanes.
//! A self input contributes the same value on another iteration. Every other
//! input must resolve to one identical value; Undefined is never discarded.
use super::*;
use std::collections::VecDeque;

pub(super) fn simplify(graph: &mut FunctionGraph, phis: &[NodeId], mut fuel: usize) {
    let mut users: HashMap<NodeId, Vec<NodeId>> = HashMap::default();
    for &phi in phis {
        if graph.resolve(phi) != phi {
            continue;
        }
        for &input in &graph.inputs(phi)[1..] {
            let Some(remaining) = fuel.checked_sub(1) else {
                return;
            };
            fuel = remaining;
            users.entry(graph.resolve(input)).or_default().push(phi);
        }
    }
    let mut pending: VecDeque<_> = phis.iter().copied().collect();
    let mut queued: HashSet<_> = phis.iter().copied().collect();
    while let Some(phi) = pending.pop_front() {
        queued.remove(&phi);
        let Some(remaining) = fuel.checked_sub(graph.inputs(phi).len()) else {
            return;
        };
        fuel = remaining;
        if graph.resolve(phi) != phi {
            continue;
        }
        let mut same = None;
        let mut different = false;
        for &input in &graph.inputs(phi)[1..] {
            let input = graph.resolve(input);
            if input == phi {
                continue;
            }
            if same.is_some_and(|old| old != input) {
                different = true;
                break;
            }
            same = Some(input);
        }
        let Some(value) = same.filter(|_| !different) else {
            continue;
        };
        let affected = users.remove(&phi).unwrap_or_default();
        let Some(remaining) = fuel.checked_sub(affected.len()) else {
            return;
        };
        fuel = remaining;
        graph.replace(phi, value);
        for &user in &affected {
            if graph.resolve(user) == user && queued.insert(user) {
                pending.push_back(user);
            }
        }
        // Future changes of the replacement must also wake the old users.
        // This preserves dependencies through an arbitrary forwarding chain.
        users.entry(value).or_default().extend(affected);
    }
}
