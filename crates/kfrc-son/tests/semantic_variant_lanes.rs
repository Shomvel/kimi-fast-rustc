//! Direct-tag enum payloads share byte-offset lanes across variants.
#[path = "support/variant_lanes.rs"]
mod variant_lanes;

use kfrc_body::Body;
use kfrc_son::{
    FunctionGraph, InterpretError, Interpreter, Opcode, Outcome, SemanticOutcome, Value,
    build_graph, lower_layout, verify_graph,
};
use variant_lanes::{expected, lanes_body};

fn layout(body: &Body) -> (FunctionGraph, FunctionGraph) {
    let mut semantic = build_graph(body).unwrap();
    verify_graph(&mut semantic).unwrap_or_else(|e| panic!("{e}\n{}", semantic.dump()));
    let mut lowered = semantic.clone();
    lower_layout(&mut lowered).unwrap_or_else(|e| panic!("{e}\n{}", lowered.dump()));
    verify_graph(&mut lowered).unwrap_or_else(|e| panic!("{e}\n{}", lowered.dump()));
    (semantic, lowered)
}

fn count(graph: &FunctionGraph, op: Opcode) -> usize {
    graph.ids().filter(|&node| graph.op(node) == op).count()
}

fn check(overlap: bool) -> FunctionGraph {
    let (semantic, lowered) = layout(&lanes_body(overlap));
    for selector in 0..8 {
        for (a, bits) in [(0, 0), (0x1ff, 0x1_2345_6789), (77, 0xfffe_dcba)] {
            let want = expected(selector, a, bits, overlap);
            assert_eq!(
                Interpreter::new(&semantic)
                    .run_values(
                        &[
                            Value::Scalar(selector),
                            Value::Scalar(a),
                            Value::Scalar(bits)
                        ],
                        &mut |name: &str, _: &[Value]| Err(InterpretError::UnknownCallee(
                            name.into()
                        ))
                    )
                    .unwrap(),
                SemanticOutcome::Return(Value::Scalar(want)),
                "S selector {selector}"
            );
            assert_eq!(
                Interpreter::new(&lowered)
                    .run(&[selector, a, bits], &mut |name: &str, _: &[u64]| Err(
                        InterpretError::UnknownCallee(name.into())
                    ))
                    .unwrap(),
                Outcome::Return(want),
                "L selector {selector}\n{}",
                lowered.dump()
            );
        }
    }
    lowered
}

#[test]
fn shared_lanes_carry_constructor_loop_and_memory_values() {
    let lowered = check(false);
    // One tag plus the byte lane at 1 and the u32 lane at 4: each is read
    // once and written once, independent of how many variants use it.
    assert_eq!(count(&lowered, Opcode::Load), 3, "{}", lowered.dump());
    assert_eq!(count(&lowered, Opcode::Store), 3, "{}", lowered.dump());
}

#[test]
fn overlapping_lanes_keep_guarded_per_variant_payloads() {
    let lowered = check(true);
    // The u16 at 4 overlaps the u32 lane, so every u32 and u16 payload keeps
    // its guarded per-variant access; the byte lane at 1 is still shared.
    assert_eq!(count(&lowered, Opcode::Load), 6, "{}", lowered.dump());
    assert_eq!(count(&lowered, Opcode::Store), 6, "{}", lowered.dump());
}
