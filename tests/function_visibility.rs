use kimi_fast_rustc::son::NodeKind;
use kimi_fast_rustc::{frontend, machine_eval, native_program};

#[test]
fn supported_visibilities_do_not_change_function_names_or_indices() {
    let graph = frontend::parse(
        "pub fn main() -> i64 { return later(40); } fn plain() -> i64 { return 0; } pub(crate) fn later(value: i64) -> i64 { return value + 2; } pub(self) fn self_visible() -> i64 { return 0; } pub(super) fn super_visible() -> i64 { return 0; }",
    )
    .unwrap();

    let names: Vec<_> = graph
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["main", "plain", "later", "self_visible", "super_visible"]
    );
    let main = graph.function_graph(&graph.functions[0]);
    assert!(
        main.iter()
            .any(|(_, node)| matches!(node.kind, NodeKind::Call(2)))
    );
}

#[test]
fn visibility_preserves_forward_calls_and_native_symbols() {
    let graph = frontend::parse(
        "pub fn main() -> i64 { return later(40); } pub(self) fn padding() -> i64 { return 0; } pub(super) fn later(value: i64) -> i64 { return value + 2; }",
    )
    .unwrap();
    let program = native_program::compile_graph(&graph).unwrap();
    let assembly = program.emit_gnu_assembly().unwrap();

    assert!(assembly.contains("\tcall later\n"), "{assembly}");
    assert!(assembly.contains(".globl main\n"), "{assembly}");
    assert!(assembly.contains("\nmain:\n"), "{assembly}");
    assert!(assembly.contains(".globl later\n"), "{assembly}");
    assert!(assembly.contains("\nlater:\n"), "{assembly}");
    assert!(!assembly.contains("pub"), "{assembly}");
    assert_eq!(&program.emit_object().unwrap()[..4], b"\x7fELF");
}

#[test]
fn unsupported_visibility_paths_and_malformed_parentheses_are_rejected() {
    for source in [
        "pub(in crate) fn main() { return; }",
        "pub(crate::inner) fn main() { return; }",
        "pub(self::inner) fn main() { return; }",
        "pub(super::inner) fn main() { return; }",
        "pub(global) fn main() { return; }",
        "pub() fn main() { return; }",
        "pub( fn main() { return; }",
        "pub(crate fn main() { return; }",
        "pub(crate, ) fn main() { return; }",
        "pub((crate)) fn main() { return; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains("function visibility"),
            "source: {source}; error: {error}"
        );
    }

    for source in [
        "pub(crate",
        "pub(crate)) fn main() { return; }",
        "pub(self) pub fn main() { return; }",
        "pub(super) struct Main;",
    ] {
        assert!(frontend::parse(source).is_err(), "source: {source}");
    }
}

#[test]
fn native_visibility_example_returns_42() {
    let graph = frontend::parse(include_str!("../examples/native_visibility.kfr")).unwrap();
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

#[test]
fn optimized_multifunction_constants_remain_inside_function_graphs() {
    let mut graph = frontend::parse(include_str!("../examples/native_visibility.kfr")).unwrap();
    kimi_fast_rustc::optimizer_pipeline::optimize(&mut graph);
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
