use kimi_fast_rustc::son::{Graph, NodeKind};
use kimi_fast_rustc::{frontend, machine_eval, native_program};

fn graph(source: &str) -> Graph {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    graph
}

fn evaluate_main(source: &str) -> i64 {
    let graph = graph(source);
    let program = native_program::compile_graph(&graph).unwrap();
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    machine_eval::evaluate(main, &[]).unwrap()
}

#[test]
fn scalar_constants_support_visibilities_radices_suffixes_and_boolean_references() {
    let value = evaluate_main(
        "const SIGNED: i64 = -0x2a;\
         const UNSIGNED: u64 = 0xffff_ffff_ffff_ffff;\
         pub const WORD: isize = 0o52isize;\
         pub(crate) const UWORD: usize = 0b10_1010usize;\
         pub(self) const ENABLED: bool = true;\
         pub(super) const DISABLED: bool = false;\
         fn main() -> i64 {\
             if ENABLED && !DISABLED {\
                 return SIGNED + WORD + UWORD as i64 + 1 + UNSIGNED as i64;\
             }\
             return 0;\
         }",
    );
    assert_eq!(value, 42);

    assert_eq!(
        evaluate_main(
            "const MINIMUM: isize = -0x8000_0000_0000_0000isize;\
             fn main() -> isize { return MINIMUM; }",
        ),
        i64::MIN
    );
}

#[test]
fn references_emit_typed_constants_and_locals_can_shadow_constant_names() {
    let parsed = graph(
        "const BASE: u64 = 0x28; const OFFSET: usize = 2usize;\
         fn main() -> u64 { return BASE + OFFSET; }",
    );
    let main = parsed.function_graph(&parsed.functions[0]);
    let addition = main
        .iter()
        .find(|(_, node)| node.kind == NodeKind::Binary(kimi_fast_rustc::son::BinaryOp::Add))
        .unwrap()
        .1;
    let left = main.node(addition.data[0]).unwrap();
    let right = main.node(addition.data[1]).unwrap();
    assert_eq!(left.kind, NodeKind::Const(40));
    assert_eq!(right.kind, NodeKind::Const(2));
    assert_eq!(
        evaluate_main(
            "const BASE: u64 = 0x28; const OFFSET: usize = 2usize;\
             fn main() -> u64 { return BASE + OFFSET; }",
        ),
        42
    );

    let shadowed =
        graph("const VALUE: i64 = 1; fn main() -> i64 { let VALUE = 42; return VALUE; }");
    let main = shadowed.function_graph(&shadowed.functions[0]);
    assert!(main.iter().all(|(_, node)| node.kind != NodeKind::Const(1)));
    assert_eq!(
        evaluate_main("const VALUE: i64 = 1; fn main() -> i64 { let VALUE = 42; return VALUE; }"),
        42
    );
}

#[test]
fn constant_items_do_not_change_function_indices_and_const_fn_stays_a_function() {
    let graph = graph(
        "const ANSWER: i64 = 42;\
         const fn identity(value: i64) -> i64 { return value; }\
         pub const UNUSED: bool = false;\
         fn main() -> i64 { return identity(ANSWER); }",
    );
    let names: Vec<_> = graph
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    assert_eq!(names, ["identity", "main"]);
    let main = graph.function_graph(&graph.functions[1]);
    assert!(main.iter().any(|(_, node)| node.kind == NodeKind::Call(0)));
    let program = native_program::compile_graph(&graph).unwrap();
    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tcall identity\n"), "{assembly}");
}

#[test]
fn constants_share_the_top_level_name_namespace_with_functions() {
    for source in [
        "const SAME: i64 = 1; const SAME: i64 = 2; fn main() { return; }",
        "const SAME: i64 = 1; fn SAME() { return; } fn main() { return; }",
        "fn SAME() { return; } const SAME: i64 = 1; fn main() { return; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(error.message.contains("duplicate item"), "{error}");
        assert!(error.message.contains("SAME"), "{error}");
    }
}

#[test]
fn unknown_constant_references_and_type_mismatches_are_diagnosed() {
    let error = frontend::parse("fn main() -> i64 { return MISSING; }").unwrap_err();
    assert!(error.message.contains("unknown variable or constant"));
    assert!(error.message.contains("MISSING"));

    for source in [
        "const VALUE: i64 = 1u64; fn main() { return; }",
        "const VALUE: u64 = 1i64; fn main() { return; }",
        "const VALUE: bool = 1; fn main() { return; }",
        "const VALUE: i64 = false; fn main() { return; }",
        "const VALUE: u64 = -1; fn main() { return; }",
        "const VALUE: u64 = 1; fn main() -> i64 { return VALUE; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(error.message.contains("type mismatch"), "{source}: {error}");
    }
}

#[test]
fn missing_constant_semicolons_have_a_specific_diagnostic() {
    for source in [
        "const VALUE: i64 = 1",
        "const VALUE: i64 = 1 fn main() { return; }",
        "const VALUE: i64 = 1 pub fn main() { return; }",
        "const VALUE: i64 = 1 const OTHER: i64 = 2; fn main() { return; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains("expected `;` after constant item"),
            "{source}: {error}"
        );
    }
}

#[test]
fn function_calls_in_constant_initializers_are_rejected_instead_of_miscompiled() {
    for initializer in ["value()", "value(1)", "value(true)"] {
        let source = format!("const VALUE: i64 = {initializer}; fn main() -> i64 {{ return 0; }}");
        let error = frontend::parse(&source).unwrap_err();
        assert!(
            error
                .message
                .contains("not supported in a constant expression"),
            "{source}: {error}"
        );
    }
}

#[test]
fn native_const_item_example_returns_42() {
    let graph = graph(include_str!("../examples/native_const_item.kfr"));
    let program = native_program::compile_graph(&graph).unwrap();
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert_eq!(machine_eval::evaluate(main, &[]), Ok(42));
    assert_eq!(&program.emit_object().unwrap()[..4], b"\x7fELF");
}
