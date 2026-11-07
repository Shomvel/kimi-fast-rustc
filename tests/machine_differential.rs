use kimi_fast_rustc::{frontend, interpreter, machine_eval, native, placement, son_optimize};

const BINARY_ARGUMENTS: &[&[i64]] = &[
    &[0, 0],
    &[1, 2],
    &[-7, 3],
    &[i64::MAX, 1],
    &[i64::MIN, 1],
    &[i64::MIN, -1],
    &[i64::MAX, i64::MAX],
    &[i64::MIN, i64::MAX],
];

fn assert_differential(name: &str, source: &str, argument_sets: &[&[i64]], require_spills: bool) {
    let original = frontend::parse(source)
        .unwrap_or_else(|error| panic!("{name}: frontend rejected a supported case: {error}"));
    let mut optimized = original.clone();
    son_optimize::optimize(&mut optimized);
    let placed = placement::place_graph(&optimized)
        .unwrap_or_else(|error| panic!("{name}: placement failed: {error}"));
    let function = native::compile_graph(&placed, name)
        .unwrap_or_else(|error| panic!("{name}: native compilation failed: {error}"));

    if require_spills {
        assert!(
            function.stack_slots > 0,
            "{name}: high-pressure case did not allocate stack slots"
        );
    }

    for &arguments in argument_sets {
        let expected = interpreter::interpret(&original, arguments).unwrap_or_else(|error| {
            panic!("{name}: original interpretation failed for {arguments:?}: {error}")
        });
        let optimized_result = interpreter::interpret(&placed, arguments);
        assert_eq!(
            optimized_result,
            Ok(expected),
            "{name}: optimized placement diverged for {arguments:?}"
        );
        let machine_result = machine_eval::evaluate(&function, arguments);
        assert_eq!(
            machine_result,
            Ok(expected),
            "{name}: MachineFunction diverged for {arguments:?}"
        );
    }
}

#[test]
fn constants_and_i64_wrapping_boundaries_match() {
    let cases = [
        ("zero", "fn main() -> i64 { return 0; }"),
        (
            "constant_expression",
            "fn main() -> i64 { let value = 40 + 2; return value; }",
        ),
        ("precedence", "fn main() -> i64 { return 7 - 3 * 2; }"),
        (
            "addition_wraps_at_maximum",
            "fn main() -> i64 { return 9223372036854775807 + 1; }",
        ),
        (
            "subtraction_wraps_at_minimum",
            "fn main() -> i64 { return -9223372036854775808 - 1; }",
        ),
        (
            "minimum_times_negative_one",
            "fn main() -> i64 { return -9223372036854775808 * -1; }",
        ),
        (
            "maximum_squared",
            "fn main() -> i64 { return 9223372036854775807 * 9223372036854775807; }",
        ),
    ];

    for (name, source) in cases {
        assert_differential(name, source, &[&[]], false);
    }
}

#[test]
fn every_supported_arithmetic_operator_matches_at_boundaries() {
    for (name, operator) in [
        ("addition", "+"),
        ("subtraction", "-"),
        ("multiplication", "*"),
    ] {
        let source =
            format!("fn main(left: i64, right: i64) -> i64 {{ return left {operator} right; }}");
        assert_differential(name, &source, BINARY_ARGUMENTS, false);
    }
}

#[test]
fn every_supported_comparison_matches_at_boundaries() {
    for (name, operator) in [
        ("equal", "=="),
        ("not_equal", "!="),
        ("less", "<"),
        ("less_equal", "<="),
        ("greater", ">"),
        ("greater_equal", ">="),
    ] {
        let source =
            format!("fn main(left: i64, right: i64) -> bool {{ return left {operator} right; }}");
        assert_differential(name, &source, BINARY_ARGUMENTS, false);
    }
}

#[test]
fn register_and_stack_parameters_match() {
    let source = "fn main(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64) -> i64 { \
                  let left = (a + b) * (c - d); \
                  let right = (e * f) + (g - h); \
                  return left - right; }";
    let arguments: &[&[i64]] = &[
        &[1, 2, 3, 4, 5, 6, 7, 8],
        &[i64::MIN, 1, i64::MAX, -1, 2, 3, 4, 5],
        &[i64::MAX, i64::MAX, i64::MIN, i64::MIN, -1, -1, 0, 0],
        &[-7, 11, -13, 17, 19, -23, 29, -31],
    ];
    assert_differential("eight_parameters", source, arguments, false);
}

#[test]
fn high_register_pressure_and_spills_match() {
    let parameters = (0..24)
        .map(|index| format!("p{index}: i64"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut expression = String::from("(p0 + p1) * (p2 - p3)");
    for group in 1..6 {
        let index = group * 4;
        let term = match group % 3 {
            0 => format!(
                "(p{index} + p{}) * (p{} - p{})",
                index + 1,
                index + 2,
                index + 3
            ),
            1 => format!(
                "(p{index} | p{}) + (p{} * p{})",
                index + 1,
                index + 2,
                index + 3
            ),
            _ => format!(
                "(p{index} - p{}) * (p{} + p{})",
                index + 1,
                index + 2,
                index + 3
            ),
        };
        expression = format!("({expression}) + ({term})");
    }
    let source = format!("fn main({parameters}) -> i64 {{ return {expression}; }}");
    let ascending = (0..24).map(|index| index as i64 - 12).collect::<Vec<_>>();
    let alternating = (0..24)
        .map(|index| {
            if index % 2 == 0 {
                i64::MIN + index as i64
            } else {
                i64::MAX - index as i64
            }
        })
        .collect::<Vec<_>>();
    let mixed = (0..24)
        .map(|index| match index % 5 {
            0 => i64::MIN,
            1 => i64::MAX,
            2 => -1,
            3 => 0,
            _ => index as i64 * 1_000_000_000_007,
        })
        .collect::<Vec<_>>();
    let arguments: &[&[i64]] = &[&ascending, &alternating, &mixed];

    assert_differential("twenty_four_parameters", &source, arguments, true);
}
