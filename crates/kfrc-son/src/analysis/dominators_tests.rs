use super::*;
use crate::{FunctionGraph, Opcode, ValueType};
use kfrc_body::{FunctionAbi, PassMode, RecordedTypes};

fn graph(size: usize) -> (FunctionGraph, Vec<NodeId>) {
    let mut graph = FunctionGraph::new(
        "dominance_queries",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Ignore,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    graph.start = graph.add_node(Opcode::Start, NodeId::NONE, &[], 0, ValueType::Control);
    let mut nodes = vec![graph.start];
    nodes.extend(
        (1..size).map(|_| graph.add_node(Opcode::Region, NodeId::NONE, &[], 0, ValueType::Control)),
    );
    (graph, nodes)
}

fn connect(graph: &mut FunctionGraph, nodes: &[NodeId], edges: &[Vec<usize>]) {
    let mut predecessors = vec![Vec::new(); nodes.len()];
    for (from, successors) in edges.iter().enumerate() {
        for &to in successors {
            predecessors[to].push(nodes[from]);
        }
    }
    for (index, inputs) in predecessors.iter().enumerate().skip(1) {
        graph.set_inputs(nodes[index], inputs);
    }
}

// This model works on logical adjacency, independently of the graph's node
// numbering, CFG reverse postorder, immediate dominators and query algorithm.
fn reached(edges: &[Vec<usize>], excluded: Option<usize>) -> Vec<bool> {
    let mut seen = vec![false; edges.len()];
    let mut pending = vec![0];
    while let Some(node) = pending.pop() {
        if excluded == Some(node) || seen[node] {
            continue;
        }
        seen[node] = true;
        pending.extend_from_slice(&edges[node]);
    }
    seen
}

fn check(graph: &FunctionGraph, nodes: &[NodeId], edges: &[Vec<usize>], queries: &[usize]) {
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    for (child, &parent) in dominators.idom.iter().enumerate().skip(1) {
        assert!(parent < child, "RPO parent {parent} must precede {child}");
    }
    let all = reached(edges, None);
    for &a in queries {
        let without_a = reached(edges, Some(a));
        for (b, &node) in nodes.iter().enumerate() {
            let expected = all[a] && all[b] && !without_a[b];
            assert_eq!(
                dominators.dominates(&cfg, nodes[a], node),
                expected,
                "logical {a} dominates {b}, actual nodes {} and {node}",
                nodes[a]
            );
            if let (Some(&a_pos), Some(&b_pos)) =
                (cfg.position.get(&nodes[a]), cfg.position.get(&node))
            {
                assert_eq!(
                    dominators.dominates_position(a_pos, b_pos),
                    expected,
                    "positions {a_pos} and {b_pos}"
                );
            }
        }
    }
}

#[test]
fn deep_sibling_chains_match_path_exclusion_in_both_query_directions() {
    const DEPTH: usize = 1024;
    let (mut graph, nodes) = graph(2 * DEPTH + 3);
    let mut edges = vec![Vec::new(); nodes.len()];
    edges[0] = vec![1, DEPTH + 1];
    for start in [1, DEPTH + 1] {
        for (index, successors) in edges.iter_mut().enumerate().skip(start).take(DEPTH - 1) {
            successors.push(index + 1);
        }
    }
    // The last two controls form an unreachable cycle.
    edges[2 * DEPTH + 1].push(2 * DEPTH + 2);
    edges[2 * DEPTH + 2].push(2 * DEPTH + 1);
    connect(&mut graph, &nodes, &edges);
    check(
        &graph,
        &nodes,
        &edges,
        &[
            0,
            1,
            DEPTH / 2,
            DEPTH,
            DEPTH + 1,
            DEPTH + DEPTH / 2,
            2 * DEPTH,
            2 * DEPTH + 1,
        ],
    );
}

#[test]
fn cyclic_cfgs_with_permuted_node_ids_match_independent_path_exclusion() {
    const SIZE: usize = 64;
    for seed in [1u64, 0x3213_8902_8923_114b, 0xa832_006d_183f_9015, u64::MAX] {
        let (mut graph, mut nodes) = graph(SIZE);
        let mut random = seed;
        let mut next = || {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            random as usize
        };
        for index in (2..SIZE).rev() {
            nodes.swap(index, 1 + next() % index);
        }
        let mut edges = vec![Vec::new(); SIZE];
        // Every node is reachable. The two-entry cycle 1 -> 3 -> 1,
        // with entry -> 2 -> 3, includes irreducible control flow.
        for (index, successors) in edges.iter_mut().enumerate().take(SIZE - 1) {
            successors.push(index + 1);
        }
        edges[0].push(2);
        edges[3].push(1);
        for successors in edges.iter_mut().skip(1) {
            for _ in 0..2 {
                successors.push(1 + next() % (SIZE - 1));
            }
        }
        connect(&mut graph, &nodes, &edges);
        check(&graph, &nodes, &edges, &(0..SIZE).collect::<Vec<_>>());
    }
}
