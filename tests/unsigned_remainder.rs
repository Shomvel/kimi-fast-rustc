use kimi_fast_rustc::son::{BinaryOp, NodeKind};
use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, optimizer_pipeline, placement};

fn optimize(source: &str) -> MachineFunction {
    let mut graph = frontend::parse(source).unwrap();
    optimizer_pipeline::optimize(&mut graph);
    graph.validate().unwrap();
    let placed = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&placed, "main").unwrap()
}

#[test]
fn unsigned_remainder_by_a_power_of_two_becomes_a_mask() {
    let mut graph = frontend::parse("fn main(value: u64) -> u64 { return value % 8; }").unwrap();
    optimizer_pipeline::optimize(&mut graph);
    assert!(graph.iter().any(|(_, node)| {
        node.kind == NodeKind::Binary(BinaryOp::And)
            && node.data.iter().any(|input| {
                matches!(
                    graph.node(*input).map(|node| node.kind),
                    Some(NodeKind::Const(7))
                )
            })
    }));
    assert!(
        !graph
            .iter()
            .any(|(_, node)| { node.kind == NodeKind::Binary(BinaryOp::URem) })
    );

    let placed = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&placed, "main").unwrap();
    for value in [-1, i64::MIN, -7, 0, 1, 7, 8, 42, i64::MAX] {
        assert_eq!(
            machine_eval::evaluate(&function, &[value]),
            Ok(((value as u64) % 8) as i64)
        );
    }
}

#[test]
fn shared_divisor_constants_keep_their_original_values() {
    let function = optimize(
        "fn main(left: u64, right: u64) -> u64 { let quotient = left / 8; let remainder = right % 8; return quotient + remainder; }",
    );
    for (left, right) in [(42, 43), (-1, -1), (i64::MIN, i64::MAX)] {
        let expected = ((left as u64) / 8).wrapping_add((right as u64) % 8) as i64;
        assert_eq!(
            machine_eval::evaluate(&function, &[left, right]),
            Ok(expected)
        );
    }
}

#[test]
fn high_bit_division_remainder_and_multiplication_are_reduced() {
    let division =
        optimize("fn main(value: u64) -> u64 { return value / 0x8000_0000_0000_0000u64; }");
    let remainder =
        optimize("fn main(value: u64) -> u64 { return value % 0x8000_0000_0000_0000u64; }");
    let multiplication =
        optimize("fn main(value: u64) -> u64 { return value * 0x8000_0000_0000_0000u64; }");
    for value in [-1, i64::MIN, -2, 0, 1, 2, i64::MAX] {
        let bits = value as u64;
        assert_eq!(
            machine_eval::evaluate(&division, &[value]),
            Ok((bits / (1u64 << 63)) as i64)
        );
        assert_eq!(
            machine_eval::evaluate(&remainder, &[value]),
            Ok((bits % (1u64 << 63)) as i64)
        );
        assert_eq!(
            machine_eval::evaluate(&multiplication, &[value]),
            Ok(bits.wrapping_mul(1u64 << 63) as i64)
        );
    }
}
