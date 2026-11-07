use kimi_fast_rustc::son::NodeKind;
use kimi_fast_rustc::{frontend, native_program};

#[test]
fn forward_calls_use_declaration_order_indices() {
    let graph = frontend::parse(
        "fn main() -> i64 { return later(40); } fn padding() -> i64 { return 0; } fn later(value: i64) -> i64 { return value + 2; }",
    )
    .unwrap();
    let main = graph.function_graph(&graph.functions[0]);
    assert!(
        main.iter()
            .any(|(_, node)| { matches!(node.kind, NodeKind::Call(2)) })
    );
    assert_eq!(graph.functions[2].name, "later");

    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall later\n"));
    let object = native_program::compile_graph_to_object(&graph).unwrap();
    assert_eq!(&object[..4], b"\x7fELF");
}

#[test]
fn self_recursion_emits_a_call_to_the_current_function() {
    let graph = frontend::parse(
        "fn factorial(value: i64) -> i64 { if value <= 1 { return 1; } return value * factorial(value - 1); } fn main() -> i64 { return factorial(5); }",
    )
    .unwrap();
    let factorial = graph.function_graph(&graph.functions[0]);
    assert!(
        factorial
            .iter()
            .any(|(_, node)| { matches!(node.kind, NodeKind::Call(0)) })
    );
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall factorial\n"));
}

#[test]
fn mutual_recursion_uses_each_forward_signature() {
    let graph = frontend::parse(
        "fn main() -> i64 { return if even(10) { 42 } else { 0 }; } fn even(value: u64) -> bool { if value == 0 { return true; } return odd(value - 1); } fn odd(value: u64) -> bool { if value == 0 { return false; } return even(value - 1); }",
    )
    .unwrap();
    let even = graph.function_graph(&graph.functions[1]);
    let odd = graph.function_graph(&graph.functions[2]);
    assert!(
        even.iter()
            .any(|(_, node)| matches!(node.kind, NodeKind::Call(2)))
    );
    assert!(
        odd.iter()
            .any(|(_, node)| matches!(node.kind, NodeKind::Call(1)))
    );
    assert!(native_program::compile_graph_to_object(&graph).is_ok());
}

#[test]
fn signature_collection_checks_types_and_skips_nested_bodies() {
    let graph = frontend::parse(
        "fn first(value: i64) -> i64 { let mut index = 0; while index < 2 { if value > 0 { index += 1; } else { index += 2; } } return second(value); } fn second(value: i64) -> i64 { return value + 2; } fn main() -> i64 { return first(40); }",
    )
    .unwrap();
    assert_eq!(graph.functions.len(), 3);

    for source in [
        "fn main() -> i64 { return later(1u64); } fn later(value: i64) -> i64 { return value; }",
        "fn main() -> i64 { return later(); } fn later(value: i64) -> i64 { return value; }",
        "fn main() -> i64 { return later(); } fn later() { return; }",
        "fn main() -> i64 { return missing(); }",
    ] {
        assert!(frontend::parse(source).is_err(), "source: {source}");
    }
}
