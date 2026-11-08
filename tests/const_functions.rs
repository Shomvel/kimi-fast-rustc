use kimi_fast_rustc::son::NodeKind;
use kimi_fast_rustc::{frontend, machine_eval, native_program};

#[test]
fn const_fn_forward_calls_use_the_prescanned_signature() {
    let graph = frontend::parse(
        "const fn main() -> i64 { return later(40); } pub const fn later(value: i64) -> i64 { return value + 2; }",
    )
    .unwrap();

    let main = graph.function_graph(&graph.functions[0]);
    assert!(
        main.iter()
            .any(|(_, node)| matches!(node.kind, NodeKind::Call(1)))
    );
    let program = native_program::compile_graph(&graph).unwrap();
    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tcall later\n"), "{assembly}");
    assert_eq!(&program.emit_object().unwrap()[..4], b"\x7fELF");
}

#[test]
fn const_fn_recursion_emits_a_runtime_call() {
    let graph = frontend::parse(
        "const fn countdown(value: i64) -> i64 { if value == 0 { return 42; } return countdown(value - 1); } pub(crate) const fn main() -> i64 { return countdown(3); }",
    )
    .unwrap();

    let countdown = graph.function_graph(&graph.functions[0]);
    assert!(
        countdown
            .iter()
            .any(|(_, node)| matches!(node.kind, NodeKind::Call(0)))
    );
    let program = native_program::compile_graph(&graph).unwrap();
    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tcall countdown\n"), "{assembly}");
    assert_eq!(&program.emit_object().unwrap()[..4], b"\x7fELF");
}

#[test]
fn const_fn_supports_all_visibilities_and_preserves_object_symbols() {
    let graph = frontend::parse(
        "pub const fn main() -> i64 { return crate_visible(); } const fn private() -> i64 { return 0; } pub(crate) const fn crate_visible() -> i64 { return 42; } pub(self) const fn self_visible() -> i64 { return 0; } pub(super) const fn super_visible() -> i64 { return 0; }",
    )
    .unwrap();

    let names: Vec<_> = graph
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "main",
            "private",
            "crate_visible",
            "self_visible",
            "super_visible"
        ]
    );
    let main = graph.function_graph(&graph.functions[0]);
    assert!(
        main.iter()
            .any(|(_, node)| matches!(node.kind, NodeKind::Call(2)))
    );

    let program = native_program::compile_graph(&graph).unwrap();
    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tcall crate_visible\n"), "{assembly}");
    let object = program.emit_object().unwrap();
    assert_eq!(&object[..4], b"\x7fELF");
    for symbol in [
        "main",
        "private",
        "crate_visible",
        "self_visible",
        "super_visible",
    ] {
        assert!(
            object
                .windows(symbol.len())
                .any(|window| window == symbol.as_bytes()),
            "missing object symbol `{symbol}`"
        );
    }
}

#[test]
fn const_fn_calls_in_initializers_are_explicitly_rejected() {
    for source in [
        "const ANSWER: i64 = answer(); fn main() -> i64 { return ANSWER; }",
        "pub const ANSWER: i64 = answer(); fn main() -> i64 { return 0; }",
        "pub(crate) const ANSWER: i64 = answer(); fn main() -> i64 { return 0; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error
                .message
                .contains("not supported in a constant expression"),
            "source: {source}; error: {error}"
        );
    }
}

#[test]
fn malformed_const_and_visibility_order_is_rejected() {
    for source in [
        "const pub fn main() { return; }",
        "const pub(crate) fn main() { return; }",
        "const const fn main() { return; }",
        "pub const pub fn main() { return; }",
        "pub fn const main() { return; }",
        "pub(crate) fn const main() { return; }",
        "fn const main() { return; }",
        "pub const(crate) fn main() { return; }",
    ] {
        assert!(frontend::parse(source).is_err(), "source: {source}");
    }
}

#[test]
fn native_const_fn_example_returns_42() {
    let graph = frontend::parse(include_str!("../examples/native_const_fn.kfr")).unwrap();
    graph.validate().unwrap();
    let program = native_program::compile_graph(&graph).unwrap();
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert_eq!(machine_eval::evaluate(main, &[]), Ok(42));
    assert_eq!(&program.emit_object().unwrap()[..4], b"\x7fELF");
}
