use kimi_fast_rustc::{frontend, native_program};

#[test]
fn unit_calls_with_arguments_work_as_statements() {
    let graph = frontend::parse(
        "fn helper(value: i64) { return; } fn main() -> i64 { helper(1); return 42; }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall helper\n"));
    assert_eq!(
        &native_program::compile_graph_to_object(&graph).unwrap()[..4],
        b"\x7fELF"
    );
}

#[test]
fn nonunit_calls_and_pure_expressions_can_be_discarded() {
    let graph = frontend::parse(
        "fn answer() -> i64 { return 42; } fn main() -> i64 { answer(); 1 + 2; 3 < 4; return answer(); }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert_eq!(assembly.matches("\tcall answer\n").count(), 2);
}

#[test]
fn unit_values_can_be_bound_and_returned_from_unit_functions() {
    let graph = frontend::parse(
        "fn helper() { return; } fn main() { let value = helper(); value; return value; }",
    )
    .unwrap();
    graph.validate().unwrap();
    assert!(native_program::compile_graph_to_object(&graph).is_ok());
}

#[test]
fn unit_values_cannot_be_used_as_numbers_or_typed_results() {
    for source in [
        "fn helper() {} fn main() -> i64 { return helper() + 1; }",
        "fn helper() {} fn main() -> bool { return helper() == helper(); }",
        "fn helper() {} fn main() -> i64 { return helper() as i64; }",
        "fn helper() {} fn main() -> i64 { return helper(); }",
        "fn helper() {} fn main(value: i64) -> i64 { return helper()(value); }",
    ] {
        assert!(frontend::parse(source).is_err(), "source: {source}");
    }
}
