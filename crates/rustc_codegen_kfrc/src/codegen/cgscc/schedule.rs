//! Iterative Kosaraju traversal. Graph and adjacency inventories are sorted;
//! reversing the SCC traversal publishes callees before callers deterministically.
use kfrc_son::{FunctionGraph, Opcode};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Component {
    pub members: Vec<String>,
    pub recursive: bool,
}

pub(super) fn postorder(
    bodies: &BTreeMap<String, FunctionGraph>,
    reachable: &BTreeSet<String>,
) -> Vec<Component> {
    let names: Vec<_> = reachable.iter().collect();
    let indices: BTreeMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(index, symbol)| (symbol.as_str(), index))
        .collect();
    let mut edges = vec![Vec::new(); names.len()];
    let mut reverse = vec![Vec::new(); names.len()];
    for (index, symbol) in names.iter().enumerate() {
        let graph = &bodies[*symbol];
        for node in graph.ids().filter(|&node| graph.op(node) == Opcode::Call) {
            let target = &graph.callees[graph.payload(node) as usize].symbol;
            if let Some(&callee) = indices.get(target.as_str()) {
                edges[index].push(callee);
            }
        }
        edges[index].sort_unstable();
        edges[index].dedup();
        for &callee in &edges[index] {
            reverse[callee].push(index);
        }
    }
    let mut seen = vec![false; names.len()];
    let mut finished = Vec::with_capacity(names.len());
    for root in 0..names.len() {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut stack = vec![(root, 0)];
        while let Some((node, next)) = stack.last_mut() {
            if *next == edges[*node].len() {
                finished.push(*node);
                stack.pop();
                continue;
            }
            let child = edges[*node][*next];
            *next += 1;
            if !seen[child] {
                seen[child] = true;
                stack.push((child, 0));
            }
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    for &root in finished.iter().rev() {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut pending = vec![root];
        let mut members = Vec::new();
        while let Some(node) = pending.pop() {
            members.push(node);
            for &parent in &reverse[node] {
                if !seen[parent] {
                    seen[parent] = true;
                    pending.push(parent);
                }
            }
        }
        members.sort_unstable();
        let recursive = members.len() > 1 || edges[root].contains(&root);
        components.push(Component {
            members: members
                .into_iter()
                .map(|index| names[index].clone())
                .collect(),
            recursive,
        });
    }
    components.reverse();
    components
}
