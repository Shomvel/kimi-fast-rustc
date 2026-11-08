use kimi_fast_rustc::son::{Graph, NodeKind};
use kimi_fast_rustc::{frontend, machine_eval, native_program};

fn graph(source: &str) -> Graph {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    graph
}

fn evaluate_typed(source: &str, ty: &str) -> i64 {
    let source = format!("{source} fn main() -> {ty} {{ return VALUE; }}");
    let graph = graph(&source);
    let program = native_program::compile_graph(&graph).unwrap();
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    machine_eval::evaluate(main, &[]).unwrap()
}

fn evaluate_i64(initializer: &str) -> i64 {
    evaluate_typed(&format!("const VALUE: i64 = {initializer};"), "i64")
}

fn evaluate_u64(initializer: &str) -> i64 {
    evaluate_typed(&format!("const VALUE: u64 = {initializer};"), "u64")
}

fn evaluate_bool(initializer: &str) -> bool {
    evaluate_typed(&format!("const VALUE: bool = {initializer};"), "bool") != 0
}

fn error(source: &str) -> String {
    frontend::parse(source).unwrap_err().message
}

#[test]
fn evaluates_arithmetic_bitwise_shift_parentheses_and_precedence() {
    for (initializer, expected) in [
        ("40 + 2", 42),
        ("46 - 4", 42),
        ("6 * 7", 42),
        ("84 / 2", 42),
        ("127 % 85", 42),
        ("0x20 | 0x0a", 42),
        ("0x3f & 0x2a", 42),
        ("0x30 ^ 0x1a", 42),
        ("21i64 << 1", 42),
        ("168i64 >> 2", 42),
        ("!(!42i64)", 42),
        ("(40 + 2)", 42),
        ("1 + 2 * 3", 7),
        ("(1 + 2) * 3", 9),
        ("0x30 | 0x0c ^ 0x02 & 0x03", 62),
        ("-7 / 2", -3),
        ("-7 % 2", -1),
        ("-8i64 >> 2", -2),
    ] {
        assert_eq!(evaluate_i64(initializer), expected, "{initializer}");
    }
}

#[test]
fn evaluates_comparisons_boolean_not_and_short_circuiting() {
    for (initializer, expected) in [
        ("2 < 3", true),
        ("2 <= 2", true),
        ("3 > 2", true),
        ("3 >= 4", false),
        ("3 == 3", true),
        ("3 != 3", false),
        ("true == !false", true),
        ("false != false", false),
        ("!false && (2 < 3) || false", true),
        ("false && (1 / 0 == 0)", false),
        ("true || (1 % 0 == 0)", true),
    ] {
        assert_eq!(evaluate_bool(initializer), expected, "{initializer}");
    }

    let message = error("const VALUE: bool = false && 1; fn main() { return; }");
    assert!(message.contains("type mismatch"), "{message}");
}

#[test]
fn preserves_signed_and_unsigned_boundaries() {
    assert_eq!(evaluate_i64("9223372036854775807i64"), i64::MAX);
    assert_eq!(evaluate_i64("-9223372036854775808i64"), i64::MIN);
    assert_eq!(
        evaluate_i64("9223372036854775807i64 - 9223372036854775806i64"),
        1
    );
    assert_eq!(evaluate_u64("18446744073709551615"), -1);
    assert_eq!(
        evaluate_u64("18446744073709551615 - 18446744073709551573"),
        42
    );
    assert_eq!(evaluate_u64("0xffff_ffff_ffff_ffffu64 >> 63u64"), 1);
    assert_eq!(evaluate_u64("1u64 << 63u64"), i64::MIN);
    assert_eq!(evaluate_u64("!0u64"), -1);
    assert!(evaluate_bool("0xffff_ffff_ffff_ffffu64 > 0"));
}

#[test]
fn resolves_forward_and_transitive_dependencies_without_emitting_runtime_operators() {
    let parsed = graph(
        "const ANSWER: i64 = (BASE + OFFSET) * 2;\
         const BASE: i64 = LIMIT - 1;\
         const OFFSET: i64 = 1;\
         const LIMIT: i64 = 21;\
         const CHECKED: bool = ANSWER == 42 && !false || false;\
         fn main() -> i64 { return ANSWER; }",
    );
    assert!(
        parsed
            .iter()
            .any(|(_, node)| node.kind == NodeKind::Const(42))
    );
    assert!(parsed.iter().all(|(_, node)| {
        matches!(
            node.kind,
            NodeKind::Start | NodeKind::Const(_) | NodeKind::Return
        )
    }));
}

#[test]
fn rejects_division_by_zero_and_arithmetic_overflow_at_compile_time() {
    for (initializer, expected) in [
        ("1i64 / 0i64", "division by zero"),
        ("1i64 % 0i64", "division by zero"),
        ("1u64 / 0u64", "division by zero"),
        ("9223372036854775807i64 + 1i64", "overflow"),
        ("-9223372036854775808i64 - 1i64", "overflow"),
        ("9223372036854775807i64 * 2i64", "overflow"),
        ("-(-9223372036854775808i64)", "overflow"),
        ("-9223372036854775808i64 / -1i64", "overflow"),
        ("-9223372036854775808i64 % -1i64", "overflow"),
    ] {
        let source = format!("const VALUE: i64 = {initializer}; fn main() {{ return; }}");
        let message = error(&source);
        assert!(message.contains(expected), "{initializer}: {message}");
    }

    for initializer in [
        "0u64 - 1u64",
        "18446744073709551615u64 + 1u64",
        "9223372036854775808u64 * 2u64",
    ] {
        let source = format!("const VALUE: u64 = {initializer}; fn main() {{ return; }}");
        assert!(error(&source).contains("overflow"), "{initializer}");
    }
}

#[test]
fn rejects_invalid_shift_counts_at_compile_time() {
    for (ty, initializer) in [
        ("i64", "1i64 << 64i64"),
        ("i64", "1i64 >> -1i64"),
        ("u64", "1u64 >> 64u64"),
        ("u64", "1u64 << 18446744073709551615u64"),
    ] {
        let source = format!("const VALUE: {ty} = {initializer}; fn main() {{ return; }}");
        let message = error(&source);
        assert!(
            message.contains("invalid shift count"),
            "{initializer}: {message}"
        );
    }
}

#[test]
fn rejects_unknown_and_cyclic_references() {
    let message = error("const VALUE: i64 = MISSING + 1; fn main() { return; }");
    assert!(message.contains("unknown constant"), "{message}");
    assert!(message.contains("MISSING"), "{message}");

    for source in [
        "const VALUE: i64 = VALUE; fn main() { return; }",
        "const FIRST: i64 = SECOND + 1; const SECOND: i64 = FIRST * 2; fn main() { return; }",
        "const FIRST: i64 = SECOND; const SECOND: i64 = THIRD; const THIRD: i64 = FIRST; fn main() { return; }",
        "const FIRST: bool = true || SECOND; const SECOND: bool = FIRST; fn main() { return; }",
        "const FIRST: bool = false && SECOND; const SECOND: bool = THIRD; const THIRD: bool = FIRST; fn main() { return; }",
    ] {
        let message = error(source);
        assert!(message.contains("cyclic constant reference"), "{message}");
    }
}

#[test]
fn preserves_const_fn_function_indices_and_local_shadowing() {
    let parsed = graph(
        "const VALUE: i64 = 40 + 2;\
         const fn identity(value: i64) -> i64 { return value; }\
         fn main() -> i64 { let VALUE = 1; return identity(VALUE); }",
    );
    let names: Vec<_> = parsed
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    assert_eq!(names, ["identity", "main"]);
    let main = parsed.function_graph(&parsed.functions[1]);
    assert!(main.iter().any(|(_, node)| node.kind == NodeKind::Call(0)));
}

#[test]
fn native_const_expression_example_returns_42() {
    let graph = graph(include_str!("../examples/native_const_expression.kfr"));
    let program = native_program::compile_graph(&graph).unwrap();
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert_eq!(machine_eval::evaluate(main, &[]), Ok(42));
    assert_eq!(&program.emit_object().unwrap()[..4], b"\x7fELF");
}
