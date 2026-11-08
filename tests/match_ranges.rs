use kimi_fast_rustc::machine_eval::MachineEvalError;
use kimi_fast_rustc::son::{CompareOp, NodeKind};
use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let main = graph
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let graph = graph.function_graph(main);
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn signed_ranges_include_zero_negative_values_and_full_type_boundaries() {
    let function = compile(
        "fn main(value: i64) -> i64 { return match value { -9223372036854775808..=-1 => 40, 0..=0 => 42, 1..=9223372036854775807 => 41, _ => 0 }; }",
    );
    for (value, expected) in [
        (i64::MIN, 40),
        (-2, 40),
        (-1, 40),
        (0, 42),
        (1, 41),
        (2, 41),
        (i64::MAX, 41),
    ] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn unsigned_ranges_use_unsigned_ordering_across_the_sign_bit() {
    let function = compile(
        "fn main(value: u64) -> u64 { return match value { 0..=0 => 40u64, 1..=9223372036854775807 => 41, 9223372036854775808..=18446744073709551615 => 42, _ => 0 }; }",
    );
    for (value, expected) in [
        (0, 40),
        (1, 41),
        (i64::MAX, 41),
        (i64::MIN, 42),
        (-2, 42),
        (-1, 42),
    ] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }

    let suffixed = compile(
        "fn main(value: usize) -> usize { return match value { 0usize..=41usize => 0usize, 42usize..=18446744073709551615usize => 42usize, _ => 1usize }; }",
    );
    assert_eq!(machine_eval::evaluate(&suffixed, &[42]), Ok(42));
    assert_eq!(machine_eval::evaluate(&suffixed, &[-1]), Ok(42));
}

#[test]
fn guards_fall_through_and_preserve_nested_matches_and_shadowing() {
    let function = compile(
        "fn main(input: i64, limit: i64) -> i64 { let input = input + 1; let answer = 40; return match input { -2..=2 if input < limit => answer + 2, 3..=5 if false => 0, _ => match input { 3..=5 => { answer + 2 }, _ => 1, }, }; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[-3, 0]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[-3, -2]), Ok(1));
    assert_eq!(machine_eval::evaluate(&function, &[2, 0]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[8, 0]), Ok(1));
}

#[test]
fn guard_calls_execute_only_after_their_pattern_matches() {
    let function = compile(
        "fn effect() -> bool { return true; } fn main(value: i64) -> i64 { return match value { 1..=2 if effect() => 42, _ => 0 }; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(0));
    assert!(matches!(
        machine_eval::evaluate(&function, &[1]),
        Err(MachineEvalError::UnsupportedCall(_))
    ));
}

#[test]
fn guard_traps_and_short_circuiting_are_control_dependent_on_the_arm() {
    let function = compile(
        "fn main(value: i64, divisor: i64) -> i64 { return match value { 0..=10 if value > 0 && 1 / divisor == 1 => 42, _ => 7 }; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[-1, 0]), Ok(7));
    assert_eq!(machine_eval::evaluate(&function, &[0, 0]), Ok(7));
    assert_eq!(machine_eval::evaluate(&function, &[1, 1]), Ok(42));
    assert_eq!(
        machine_eval::evaluate(&function, &[1, 0]),
        Err(MachineEvalError::DivisionByZero)
    );
}

#[test]
fn range_lowering_uses_unsigned_compares_control_regions_and_result_phi() {
    let graph = frontend::parse(
        "fn main(value: u64, flag: bool) -> u64 { return match value { 0..=42 if flag => 42u64, _ => 0 }; }",
    )
    .unwrap();
    graph.validate().unwrap();
    assert!(graph.iter().any(|(_, node)| {
        node.kind == NodeKind::Compare(CompareOp::UGe) && node.data.len() == 2
    }));
    assert!(graph.iter().any(|(_, node)| {
        node.kind == NodeKind::Compare(CompareOp::ULe) && node.data.len() == 2
    }));
    assert_eq!(
        graph
            .iter()
            .filter(|(_, node)| node.kind == NodeKind::If)
            .count(),
        3
    );
    assert!(graph.iter().any(|(_, node)| {
        node.kind == NodeKind::Phi && node.control.len() == 2 && node.data.len() == 2
    }));
}

#[test]
fn reversed_exclusive_and_non_integer_ranges_are_rejected() {
    for source in [
        "fn main(value: i64) -> i64 { return match value { 2..=1 => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { -1..=-2 => 1, _ => 0 }; }",
        "fn main(value: u64) -> i64 { return match value { 18446744073709551615..=0 => 1, _ => 0 }; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(error.message.contains("less than or equal"), "{error}");
    }

    let exclusive =
        frontend::parse("fn main(value: i64) -> i64 { return match value { 0..2 => 1, _ => 0 }; }")
            .unwrap_err();
    assert!(exclusive.message.contains("only inclusive `..=`"));

    let boolean = frontend::parse(
        "fn main(value: bool) -> i64 { return match value { false..=true => 1, _ => 0 }; }",
    )
    .unwrap_err();
    assert!(boolean.message.contains("integer scrutinee"));
}

#[test]
fn overlapping_ranges_and_literals_are_rejected() {
    for source in [
        "fn main(value: i64) -> i64 { return match value { 1 => 0, 0..=2 => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { 0..=2 => 0, 1 => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { 0..=10 => 0, 10..=20 => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { -10..=-5 => 0, -5..=-1 => 1, _ => 0 }; }",
        "fn main(value: u64) -> i64 { return match value { 18446744073709551605..=18446744073709551615 => 0, 18446744073709551615..=18446744073709551615 => 1, _ => 0 }; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(error.message.contains("overlapping"), "{source}: {error}");
    }

    let duplicate = frontend::parse(
        "fn main(value: i64) -> i64 { return match value { 0..=0 => 0, 0..=0 => 1, _ => 0 }; }",
    )
    .unwrap_err();
    assert!(duplicate.message.contains("overlapping"));
}

#[test]
fn range_endpoint_and_guard_types_are_checked() {
    for source in [
        "fn main(value: i64) -> i64 { return match value { 0u64..=2 => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { 0..=2u64 => 1, _ => 0 }; }",
        "fn main(value: u64) -> i64 { return match value { -2..=1 => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { 0..=true => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { 0..=2 if 1 => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { 0 if 1 => 1, _ => 0 }; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains("type mismatch")
                || error.message.contains("unsigned suffix")
                || error.message.contains("expected integer"),
            "{source}: {error}"
        );
    }
}

#[test]
fn guarded_wildcard_is_rejected_as_non_exhaustive() {
    let error =
        frontend::parse("fn main(value: i64) -> i64 { return match value { _ if true => 42 }; }")
            .unwrap_err();
    assert!(error.message.contains("non-exhaustive"));
}

#[test]
fn native_match_range_example_returns_42() {
    let function = compile(include_str!("../examples/native_match_range.kfr"));
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}
