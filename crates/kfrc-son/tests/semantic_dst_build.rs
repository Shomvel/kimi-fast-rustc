//! DST place addresses preserve the owner and carry slice metadata separately.
use kfrc_body::*;
use kfrc_son::{
    Interpreter, Opcode, Outcome, SemanticOutcome, Value, build_graph, lower_layout, verify_graph,
};

#[path = "support/dst.rs"]
mod dst;
use dst::{nested_tail_body, string_tail_body};

#[test]
fn nested_slice_tail_preserves_metadata_and_prefix_writes_in_s_and_l() {
    let (body, inner, slice) = nested_tail_body(true);
    let mut graph = build_graph(&body).unwrap();
    verify_graph(&mut graph).unwrap();
    for pointee in [inner, slice] {
        assert!(graph.ids().any(|node| graph.op(node) == Opcode::FieldAddr && graph.rust_type(node).is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::PlaceAddress { pointee: actual } if *actual == pointee))));
    }
    assert_eq!(
        Interpreter::new(&graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
    lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(
        Interpreter::new(&graph)
            .run(&[], &mut |_: &str, _: &[u64]| unreachable!())
            .unwrap(),
        Outcome::Return(42)
    );
}

#[test]
fn unsized_field_without_static_tail_evidence_is_rejected() {
    let (body, _, _) = nested_tail_body(false);
    assert!(
        build_graph(&body)
            .unwrap_err()
            .to_string()
            .contains("producer-proven static slice/string tail")
    );
}

#[test]
fn string_tail_reference_keeps_metadata_and_real_data_offset() {
    let mut graph = build_graph(&string_tail_body()).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(
        Interpreter::new(&graph)
            .run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(109))
    );
    lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(
        Interpreter::new(&graph)
            .run(&[], &mut |_: &str, _: &[u64]| unreachable!())
            .unwrap(),
        Outcome::Return(109)
    );
}
