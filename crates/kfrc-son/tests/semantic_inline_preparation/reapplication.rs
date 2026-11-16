//! Metadata becomes visible only after a helper forwards a whole parameter.
//! Existing call-boundary observations must still precede refined execution.
use super::*;
use kfrc_body::{
    AggregateLayout, FieldLayout, IntegerWidth, Mutability, SizeAlign, TyId, TypeShape,
};
use kfrc_son::{ComparePredicate, FunctionGraph, InterpretError, SemanticOutcome, ValueType};

fn slice_types(f: &mut Function) -> (TyId, TyId) {
    let byte = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let slice = f
        .graph
        .oracle
        .intern(TypeShape::Slice { element: byte }, None);
    let reference = f.graph.oracle.intern(
        TypeShape::Reference {
            mutability: Mutability::Not,
            pointee: slice,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    let unit = f
        .graph
        .oracle
        .intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let data = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: unit,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    f.graph.oracle.record_aggregate(
        reference,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: data,
                    offset: 0,
                },
                FieldLayout {
                    ty: f.word,
                    offset: 8,
                },
            ],
            variants: Vec::new(),
        },
    );
    (reference, data)
}

fn slice_argument(f: &mut Function, parameter: usize) {
    let (slice, _) = slice_types(f);
    let ValueType::Tuple(mut outputs) = f.graph.value_type(f.graph.start).clone() else {
        unreachable!()
    };
    outputs[parameter + 2] = ValueType::Rust(slice);
    f.graph
        .set_value_type(f.graph.start, ValueType::Tuple(outputs));
    f.graph
        .set_value_type(f.args[parameter], ValueType::Rust(slice));
}

fn reader() -> Function {
    let mut f = Function::new("metadata_reader", 1);
    slice_argument(&mut f, 0);
    let length = f.node(Opcode::Field, &[f.args[0]], 1, f.word);
    f.ret(length);
    f.verify();
    f
}

fn wrapper(child: &FunctionGraph) -> Function {
    let mut f = Function::new("metadata_wrapper", 2);
    slice_argument(&mut f, 1);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let limit = f.constant(128);
    let condition = f.node(
        Opcode::Compare,
        &[f.args[0], limit],
        ComparePredicate::UnsignedLess as u32,
        boolean,
    );
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let entry_effect = f.effect;
    f.control = projection(&mut f.graph, branch, 0, ValueType::Control);
    let value = f.call(child, &[f.args[1]]);
    f.ret(value);
    f.control = projection(&mut f.graph, branch, 1, ValueType::Control);
    f.effect = entry_effect;
    let mut value = f.call_named("touch", &child.abi, &[f.args[0]]);
    for step in 0..40 {
        let constant = f.constant(step);
        value = f.node(Opcode::Add, &[value, constant], 0, f.word);
    }
    f.ret(value);
    f.verify();
    assert!(f.graph.live_count() > 64 && f.graph.len() <= 512);
    f
}

fn program(poison: Option<bool>) -> (Function, HashMap<String, FunctionGraph>) {
    let reader = reader();
    let source = wrapper(&reader.graph);
    let graphs = HashMap::from_iter([
        (source.graph.symbol.clone(), source.graph.clone()),
        (reader.graph.symbol.clone(), reader.graph),
    ]);
    let mut f = Function::new("metadata_caller", 0);
    let (slice, data_type) = slice_types(&mut f);
    let storage = f.address("metadata_storage", true);
    let data = if poison == Some(false) {
        f.node(Opcode::Undefined, &[], 0, data_type)
    } else {
        f.node(Opcode::PointerCast, &[storage], 0, data_type)
    };
    let four = f.constant(4);
    let length = if poison == Some(true) {
        let undefined = f.node(Opcode::Undefined, &[], 0, f.word);
        let zero = f.constant(0);
        let masked = f.node(Opcode::BitAnd, &[undefined, zero], 0, f.word);
        f.node(Opcode::BitOr, &[masked, four], 0, f.word)
    } else {
        four
    };
    let slice = f.node(Opcode::WidePtr, &[data, length], 0, slice);
    let tag = f.constant(32);
    let value = f.call(&source.graph, &[tag, slice]);
    f.ret(value);
    f.verify();
    (f, graphs)
}

#[test]
fn exact_metadata_fact_refines_a_field_first_exposed_by_nested_splicing() {
    let (mut f, graphs) = program(None);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    let work = &stats.preparation;
    assert!(work.fact_reapplication_fields_replaced > 0);
    assert_eq!(
        work.fact_reapplication_fields_replaced,
        work.fact_reapplication_nodes_allocated
    );
    assert!(work.fact_reapplication_node_visits <= work.fact_reapplication_visit_budget);
    assert_eq!(
        work.clone_budget_used,
        work.source_nodes_cloned
            + work.transaction_nodes_cloned
            + work.splice_nodes_allocated
            + work.fact_reapplication_nodes_allocated
    );
    assert!(work.clone_budget_used <= 2000);
    assert!(calls(&f.graph).is_empty());
    f.verify();
    equivalent(&before, &f.graph, &graphs, &[]);
    assert_eq!(
        execute(&f.graph, &graphs, &[]),
        (Ok(SemanticOutcome::Return(Value::Scalar(4))), Vec::new())
    );
}

#[test]
fn refining_known_metadata_preserves_poison_in_that_field_or_its_sibling() {
    for poison in [false, true] {
        let (mut f, graphs) = program(Some(poison));
        let before = f.graph.clone();
        let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
        assert!(stats.preparation.fact_reapplication_fields_replaced > 0);
        f.verify();
        equivalent(&before, &f.graph, &graphs, &[]);
        let (result, trace) = execute(&f.graph, &graphs, &[]);
        assert!(matches!(result, Err(InterpretError::PoisonObserved(_))));
        assert!(trace.is_empty());
    }
}
