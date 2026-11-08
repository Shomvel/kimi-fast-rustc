use kimi_fast_rustc::son::NodeKind;
use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn signed_match_supports_negative_literals_blocks_and_trailing_commas() {
    let function = compile(
        "fn main(value: i64) -> i64 { return match value { -1 => 40, 0 => { 40 + 1 }, 42 => 42, -9223372036854775808 => 43, _ => 0, }; }",
    );
    for (value, expected) in [(-1, 40), (0, 41), (42, 42), (i64::MIN, 43), (1, 0)] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn unsigned_and_word_alias_patterns_preserve_the_full_bit_range() {
    let unsigned = compile(
        "fn main(value: u64) -> u64 { let result = match value { 0 => 40u64, 18446744073709551615u64 => 42, _ => 1 }; return result; }",
    );
    assert_eq!(machine_eval::evaluate(&unsigned, &[0]), Ok(40));
    assert_eq!(machine_eval::evaluate(&unsigned, &[-1]), Ok(42));
    assert_eq!(machine_eval::evaluate(&unsigned, &[1]), Ok(1));

    let signed_word = compile(
        "fn main(value: isize) -> isize { return match value { -2isize => 42isize, _ => 0isize }; }",
    );
    assert_eq!(machine_eval::evaluate(&signed_word, &[-2]), Ok(42));
    assert_eq!(machine_eval::evaluate(&signed_word, &[2]), Ok(0));

    let unsigned_word = compile(
        "fn main(value: usize) -> usize { return match value { 42usize => 42usize, _ => 0usize }; }",
    );
    assert_eq!(machine_eval::evaluate(&unsigned_word, &[42]), Ok(42));
    assert_eq!(machine_eval::evaluate(&unsigned_word, &[0]), Ok(0));
}

#[test]
fn bool_patterns_and_tail_match_select_the_expected_value() {
    let function =
        compile("fn main(flag: bool) -> i64 { match flag { true => 42, false => 0, _ => 1 } }");
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(0));
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(42));
}

#[test]
fn tail_detection_stops_at_the_current_function_and_wildcard_only_matches_work() {
    let graph = frontend::parse(
        "fn main(value: i64) -> i64 { match value { 1 => 42, _ => 0 } } fn later(value: i64) -> i64 { return value; }",
    )
    .unwrap();
    graph.validate().unwrap();

    let wildcard = compile("fn main(value: i64) -> i64 { match value { _ => 42 } }");
    assert_eq!(machine_eval::evaluate(&wildcard, &[100]), Ok(42));
}

#[test]
fn nested_match_and_shadowed_bindings_keep_their_values() {
    let function = compile(
        "fn main(value: i64) -> i64 { let value = value + 1; let result = 40; match value { 1 => result + 2, _ => match value { 3 => { result }, _ => 0, }, } }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[2]), Ok(40));
    assert_eq!(machine_eval::evaluate(&function, &[8]), Ok(0));
}

#[test]
fn unsuffixed_arm_results_unify_with_unsigned_results() {
    let function = compile(
        "fn main(value: u64) -> u64 { return match value { 0 => 40, 1 => 41, 2 => 42u64, _ => 0 }; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[2]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[9]), Ok(0));
}

#[test]
fn match_lowers_to_if_chains_and_a_multiway_phi() {
    let graph = frontend::parse(
        "fn main(value: i64) -> i64 { return match value { 0 => 10, 1 => 20, _ => 30 }; }",
    )
    .unwrap();
    graph.validate().unwrap();
    assert_eq!(
        graph
            .iter()
            .filter(|(_, node)| node.kind == NodeKind::If)
            .count(),
        2
    );
    assert!(graph.iter().any(|(_, node)| {
        node.kind == NodeKind::Phi && node.control.len() == 3 && node.data.len() == 3
    }));
}

#[test]
fn native_match_example_returns_42() {
    let function = compile(include_str!("../examples/native_match.kfr"));
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn pattern_and_arm_types_are_checked() {
    for source in [
        "fn main(value: i64) -> i64 { return match value { true => 1, _ => 0 }; }",
        "fn main(value: bool) -> i64 { return match value { 0 => 1, _ => 0 }; }",
        "fn main(value: u64) -> i64 { return match value { -1 => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { 1u64 => 1, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { 0 => 1, _ => true }; }",
        "fn main(value: i64) -> i64 { return match value { 0 => 1i64, _ => 0u64 }; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains("type mismatch") || error.message.contains("unsigned suffix"),
            "source: {source}; error: {error}"
        );
    }
}

#[test]
fn wildcard_is_required_last_and_unique() {
    let missing =
        frontend::parse("fn main(value: i64) -> i64 { return match value { 0 => 1, 1 => 2 }; }")
            .unwrap_err();
    assert!(missing.message.contains("requires a final `_`"));

    for source in [
        "fn main(value: i64) -> i64 { return match value { _ => 0, 1 => 2 }; }",
        "fn main(value: i64) -> i64 { return match value { _ => 0, _ => 1 }; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(error.message.contains("last and unique"));
    }
}

#[test]
fn duplicate_literals_are_rejected_across_equal_spellings() {
    for source in [
        "fn main(value: i64) -> i64 { return match value { -1 => 1, -1 => 2, _ => 0 }; }",
        "fn main(value: i64) -> i64 { return match value { 0x2a => 1, 42 => 2, _ => 0 }; }",
        "fn main(value: bool) -> i64 { return match value { true => 1, true => 2, _ => 0 }; }",
        "fn main(value: u64) -> i64 { return match value { 18446744073709551615 => 1, 18446744073709551615u64 => 2, _ => 0 }; }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains("duplicate literal"),
            "source: {source}"
        );
    }
}

#[test]
fn unsupported_patterns_and_match_as_an_identifier_are_rejected() {
    for (source, expected) in [
        (
            "fn main(value: i64) -> i64 { return match value { 0..2 => 1, _ => 0 }; }",
            "range patterns",
        ),
        (
            "fn main(value: i64) -> i64 { return match value { 0 if true => 1, _ => 0 }; }",
            "guards",
        ),
        (
            "fn main(value: i64) -> i64 { return match value { (0) => 1, _ => 0 }; }",
            "tuple and enum patterns",
        ),
        (
            "fn main(value: i64) -> i64 { return match value { Some => 1, _ => 0 }; }",
            "tuple and enum patterns",
        ),
        ("fn match() -> i64 { return 42; }", "reserved keyword"),
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains(expected),
            "source: {source}; error: {error}"
        );
    }
}
