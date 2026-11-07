use kimi_fast_rustc::interpreter::{InterpretError, TrapKind};
use kimi_fast_rustc::machine_eval::MachineEvalError;
use kimi_fast_rustc::{frontend, interpreter, machine_eval, native, optimizer_pipeline, placement};

const BINARY_ARGUMENTS: &[&[i64]] = &[
    &[42, 7],
    &[-42, 7],
    &[42, -7],
    &[-42, -7],
    &[-43, 7],
    &[43, -7],
    &[0, 7],
    &[1, 1],
    &[-1, -1],
    &[i64::MAX, 1],
    &[i64::MAX, -1],
    &[i64::MIN, 1],
    &[i64::MIN, 2],
    &[i64::MIN, -2],
    &[i64::MIN, i64::MAX],
    &[i64::MAX, i64::MIN],
    &[i64::MIN, i64::MIN],
    &[i64::MAX, i64::MAX],
];

fn assert_native_idiv(name: &str, function: &kimi_fast_rustc::x86::MachineFunction) {
    let assembly = function
        .emit_gnu_assembly()
        .unwrap_or_else(|error| panic!("{name}: GNU assembly emission failed: {error}"));
    assert!(
        assembly.contains("\tcqo\n\tidiv "),
        "{name}: GNU assembly does not contain cqo/idiv:\n{assembly}"
    );

    let code = function
        .emit_machine_code()
        .unwrap_or_else(|error| panic!("{name}: direct machine-code emission failed: {error}"));
    let has_encoded_idiv = code.windows(5).any(|bytes| {
        bytes.starts_with(&[0x48, 0x99])
            && matches!(bytes[2], 0x48 | 0x49)
            && bytes[3] == 0xf7
            && ((bytes[4] >> 3) & 7) == 7
    });
    assert!(
        has_encoded_idiv,
        "{name}: direct machine code does not contain cqo followed by 64-bit idiv: {code:02x?}"
    );
}

fn assert_differential(name: &str, source: &str, argument_sets: &[&[i64]], require_spills: bool) {
    let original = frontend::parse(source)
        .unwrap_or_else(|error| panic!("{name}: frontend rejected the source: {error}"));
    let mut optimized = original.clone();
    optimizer_pipeline::optimize(&mut optimized);
    let placed = placement::place_graph(&optimized)
        .unwrap_or_else(|error| panic!("{name}: placement failed: {error}"));
    let function = native::compile_graph(&placed, name)
        .unwrap_or_else(|error| panic!("{name}: native compilation failed: {error}"));

    if require_spills {
        assert!(
            function.stack_slots > 0,
            "{name}: high-pressure case did not allocate spill slots"
        );
    }
    assert_native_idiv(name, &function);

    for &arguments in argument_sets {
        let expected = interpreter::interpret(&original, arguments).unwrap_or_else(|error| {
            panic!("{name}: original interpretation failed for {arguments:?}: {error}")
        });
        assert_eq!(
            interpreter::interpret(&placed, arguments),
            Ok(expected),
            "{name}: optimizer/placement diverged for {arguments:?}"
        );
        assert_eq!(
            machine_eval::evaluate(&function, arguments),
            Ok(expected),
            "{name}: native machine function diverged for {arguments:?}"
        );
    }
}

fn assert_interpreter_trap(
    name: &str,
    stage: &str,
    result: Result<i64, InterpretError>,
    expected: TrapKind,
) {
    match result {
        Err(InterpretError::Trap { kind, .. }) => {
            assert_eq!(kind, expected, "{name}: wrong {stage} trap")
        }
        other => panic!("{name}: {stage} did not report {expected:?}: {other:?}"),
    }
}

#[test]
fn quotient_and_remainder_match_for_values_and_boundaries() {
    for (name, operator) in [("division", "/"), ("remainder", "%")] {
        let source =
            format!("fn main(left: i64, right: i64) -> i64 {{ return left {operator} right; }}");
        assert_differential(name, &source, BINARY_ARGUMENTS, false);
    }
}

#[test]
fn division_by_zero_and_overflow_remain_traps() {
    let cases = [
        (
            "division_by_zero",
            "/",
            [1, 0],
            TrapKind::DivisionByZero,
            MachineEvalError::DivisionByZero,
        ),
        (
            "remainder_by_zero",
            "%",
            [1, 0],
            TrapKind::RemainderByZero,
            MachineEvalError::DivisionByZero,
        ),
        (
            "division_overflow",
            "/",
            [i64::MIN, -1],
            TrapKind::DivisionOverflow,
            MachineEvalError::DivisionOverflow,
        ),
        (
            "remainder_overflow",
            "%",
            [i64::MIN, -1],
            TrapKind::RemainderOverflow,
            MachineEvalError::DivisionOverflow,
        ),
    ];

    for (name, operator, arguments, interpreter_trap, native_error) in cases {
        let source =
            format!("fn main(left: i64, right: i64) -> i64 {{ return left {operator} right; }}");
        let original = frontend::parse(&source)
            .unwrap_or_else(|error| panic!("{name}: frontend rejected the source: {error}"));
        let mut optimized = original.clone();
        optimizer_pipeline::optimize(&mut optimized);
        let placed = placement::place_graph(&optimized)
            .unwrap_or_else(|error| panic!("{name}: placement failed: {error}"));
        let function = native::compile_graph(&placed, name)
            .unwrap_or_else(|error| panic!("{name}: native compilation failed: {error}"));
        assert_native_idiv(name, &function);

        assert_interpreter_trap(
            name,
            "original interpreter",
            interpreter::interpret(&original, &arguments),
            interpreter_trap,
        );
        assert_interpreter_trap(
            name,
            "optimized interpreter",
            interpreter::interpret(&placed, &arguments),
            interpreter_trap,
        );
        assert_eq!(
            machine_eval::evaluate(&function, &arguments),
            Err(native_error),
            "{name}: native evaluation silently produced a value or returned the wrong error"
        );
    }
}

#[test]
fn quotient_and_remainder_match_with_spills() {
    let parameters = (0..24)
        .map(|index| format!("p{index}: i64"))
        .collect::<Vec<_>>()
        .join(", ");
    let ordinary = (0..24).map(|index| index as i64 - 12).collect::<Vec<_>>();
    let negative = (0..24)
        .map(|index| {
            if index % 2 == 0 {
                -(index as i64) - 3
            } else {
                index as i64 * 3 + 1
            }
        })
        .collect::<Vec<_>>();
    let boundaries = (0..24)
        .map(|index| match index % 4 {
            0 => i64::MIN + index as i64,
            1 => i64::MAX - index as i64,
            2 => -1,
            _ => 1,
        })
        .collect::<Vec<_>>();
    let mut minimum_by_one = boundaries.clone();
    minimum_by_one[0] = i64::MIN;
    minimum_by_one[1] = 1;
    let mut maximum_by_negative_one = boundaries;
    maximum_by_negative_one[0] = i64::MAX;
    maximum_by_negative_one[1] = -1;
    let arguments: &[&[i64]] = &[
        &ordinary,
        &negative,
        &minimum_by_one,
        &maximum_by_negative_one,
    ];

    for (name, operator) in [("spilled_division", "/"), ("spilled_remainder", "%")] {
        let mut expression = format!("(p0 {operator} p1) + (p2 * p3)");
        for group in 1..6 {
            let index = group * 4;
            expression = format!(
                "({expression}) + ((p{index} + p{}) * (p{} - p{}))",
                index + 1,
                index + 2,
                index + 3
            );
        }
        let source = format!("fn main({parameters}) -> i64 {{ return {expression}; }}");
        assert_differential(name, &source, arguments, true);
    }
}
