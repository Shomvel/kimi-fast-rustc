use super::*;
use kfrc_body::{FunctionAbi, IntegerWidth, RecordedTypes, SizeAlign};

fn identity() -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut graph = FunctionGraph::new(
        "pipeline_state",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Specialized;
    graph.start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    let mut projection =
        |index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[graph.start], index, ty);
    let control = projection(0, ValueType::Control);
    let effect = projection(1, ValueType::Effect);
    let argument = projection(2, ValueType::Rust(word));
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, argument],
        0,
        ValueType::Control,
    );
    graph.state = GraphState::Sealed;
    graph
}

#[test]
fn publication_distinguishes_internal_trust_from_full_verification() {
    for verify in [false, true] {
        let mut graph = identity();
        assert!(matches!(
            crate::lower_layout(&mut graph),
            Err(crate::LayoutError::NotVerified)
        ));
        finish_pipeline_output(&mut graph, verify).unwrap();
        assert_eq!(
            graph.state,
            if verify {
                GraphState::Verified
            } else {
                GraphState::Trusted
            }
        );
        crate::lower_layout(&mut graph).unwrap();
        assert_eq!(graph.level, GraphLevel::Layout);
        assert_eq!(graph.state, GraphState::Sealed);
        finish_pipeline_output(&mut graph, verify).unwrap();
        assert!(graph.state.is_ready_for_lowering());
        // Both publication modes produce the same graph invariants.
        verify_graph(&mut graph).unwrap();
        assert_eq!(graph.state, GraphState::Verified);
    }
}

#[test]
fn edits_revoke_trust_and_failed_verification_never_restores_it() {
    let mut graph = identity();
    finish_pipeline_output(&mut graph, false).unwrap();
    graph.set_opcode(graph.start, Opcode::Undefined);
    assert_eq!(graph.state, GraphState::Sealed);
    assert!(matches!(
        crate::lower_layout(&mut graph),
        Err(crate::LayoutError::NotVerified)
    ));
    assert!(finish_pipeline_output(&mut graph, true).is_err());
    assert_eq!(graph.state, GraphState::Sealed);
}
