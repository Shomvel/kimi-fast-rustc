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
        for (a, bits) in [
            (0, 0),
            (0x1ff, 0x1_2345_6789),
            (77, 0xfffe_dcba),
            (u64::MAX, u64::MAX),
            (0x80, 0x1_0000),
            (0x101, 0xffff),
        ] {
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
    // The three nonoverlapping lanes have exact private writers. Their
    // round trips disappear after layout; overlapping lanes below cannot.
    assert_eq!(count(&lowered, Opcode::Load), 0, "{}", lowered.dump());
    assert_eq!(count(&lowered, Opcode::Store), 0, "{}", lowered.dump());
}

#[test]
fn overlapping_lanes_keep_physical_storage_and_variant_values() {
    let lowered = check(true);
    // Overlapping u16/u32 representations retain their common physical slot.
    // Redundant tag/byte reads may disappear after forwarding or threading;
    // the S/L matrix above checks every resulting variant's payload value.
    assert_eq!(count(&lowered, Opcode::StackSlot), 1, "{}", lowered.dump());
    assert_eq!(count(&lowered, Opcode::Store), 6, "{}", lowered.dump());
}
