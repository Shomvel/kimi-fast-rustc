use kimi_fast_rustc::son::{Graph, NodeKind};
use kimi_fast_rustc::x86::VirtualInstr;
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

#[test]
fn acyclic_loop_phi_edges_do_not_allocate_temporaries() {
    let graph = frontend::parse(
        "fn main() -> i64 { let mut sum = 0; let mut i = 0; while i < 5 { sum += i; i += 1; } return sum + 32; }",
    )
    .unwrap();
    let maximum = graph.ids().map(|id| id.0).max().unwrap_or(0);
    let instructions = native::lower_graph(&graph).unwrap();
    for instruction in instructions {
        if let VirtualInstr::Mov { dst, .. } = instruction {
            assert!(dst.0 <= maximum, "unexpected temporary {dst:?}");
        }
    }

    let graph = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&graph, "main").unwrap();
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn cyclic_phi_edges_keep_parallel_copies() {
    let mut graph = Graph::new();
    let start = graph.start();
    let one = graph.constant(1);
    let two = graph.constant(2);
    let header = graph.region(vec![start]);
    let a = graph.phi(vec![start], vec![one]);
    let b = graph.phi(vec![start], vec![two]);
    let condition = graph.constant(1);
    let branch = graph.if_(header, condition);
    let body = graph.projection(branch, 0);
    let back = graph.jump(body);
    graph.node_mut(header).unwrap().control.push(back);
    for (phi, source) in [(a, b), (b, a)] {
        let node = graph.node_mut(phi).unwrap();
        node.control.push(back);
        node.data.push(source);
    }
    let exit = graph.projection(branch, 1);
    graph.return_(exit, Some(a));
    graph.validate().unwrap();

    let maximum = graph.ids().map(|id| id.0).max().unwrap_or(0);
    let instructions = native::lower_graph(&graph).unwrap();
    assert!(instructions.iter().any(|instruction| matches!(
        instruction,
        VirtualInstr::Mov { dst, .. } if dst.0 > maximum
    )));
    assert_eq!(graph.node(back).map(|node| node.kind), Some(NodeKind::Jump));
}
