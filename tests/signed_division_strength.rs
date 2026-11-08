use kimi_fast_rustc::machine_eval::MachineEvalError;
use kimi_fast_rustc::son::{BinaryOp, Graph};
use kimi_fast_rustc::x86::{MachineFunction, MachineInstr};
use kimi_fast_rustc::{
    frontend, machine_eval, native, native_object, optimizer_pipeline, placement,
};

const VALUES: [i64; 23] = [
    i64::MIN,
    i64::MIN + 1,
    i64::MIN + 2,
    i64::MIN + 3,
    -9,
    -8,
    -7,
    -5,
    -4,
    -3,
    -2,
    -1,
    0,
    1,
    2,
    3,
    4,
    5,
    7,
    8,
    9,
    i64::MAX - 1,
    i64::MAX,
];

fn compile(source: &str) -> MachineFunction {
    let mut graph = frontend::parse(source).unwrap();
    optimizer_pipeline::optimize(&mut graph);
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

fn assert_reduced(function: &MachineFunction, shift: u8) {
    assert!(
        !function.instructions.iter().any(|instruction| matches!(
            instruction,
            MachineInstr::IDiv { .. } | MachineInstr::Cqo
        ))
    );
    if shift == 0 {
        return;
    }

    let sign_shift = function
        .instructions
        .iter()
        .position(|instruction| matches!(instruction, MachineInstr::ShrImm { count: 63, .. }))
        .unwrap();
    assert!(matches!(
        function.instructions[sign_shift + 1],
        MachineInstr::LShrImm { count, .. } if count == 64 - shift
    ));
    assert!(matches!(
        function.instructions[sign_shift + 2],
        MachineInstr::Add { .. }
    ));
    assert!(matches!(
        function.instructions[sign_shift + 3],
        MachineInstr::ShrImm { count, .. } if count == shift
    ));
}

#[test]
fn positive_powers_avoid_idiv_and_round_toward_zero() {
    for (divisor, shift) in [
        (2, 1),
        (4, 2),
        (8, 3),
        (1_i64 << 31, 31),
        (1_i64 << 32, 32),
        (1_i64 << 62, 62),
    ] {
        let function = compile(&format!(
            "fn main(value: i64) -> i64 {{ return value / {divisor}; }}"
        ));
        assert_reduced(&function, shift);
        function.emit_machine_code().unwrap();
        for value in VALUES {
            assert_eq!(
                machine_eval::evaluate(&function, &[value]),
                Ok(value / divisor),
                "value={value}, divisor={divisor}"
            );
        }
    }
}

#[test]
fn native_division_by_one_is_a_copy() {
    let mut graph = Graph::new();
    let start = graph.start();
    let value = graph.parameter(0);
    let one = graph.constant(1);
    let quotient = graph.binary(BinaryOp::Div, value, one);
    graph.return_(start, Some(quotient));
    let function = native::compile_graph(&graph, "main").unwrap();
    assert_reduced(&function, 0);
    for value in VALUES {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(value));
    }
}

#[test]
fn two_to_63_bit_pattern_remains_signed_division_by_a_negative() {
    let function = compile("fn main(value: i64) -> i64 { return value / -9223372036854775808; }");
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::IDiv { .. }))
    );
    for (value, expected) in [
        (i64::MIN, 1),
        (i64::MIN + 1, 0),
        (-1, 0),
        (0, 0),
        (1, 0),
        (i64::MAX, 0),
    ] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn dynamic_divisors_keep_all_signed_division_traps() {
    let function = compile("fn main(value: i64, divisor: i64) -> i64 { return value / divisor; }");
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::IDiv { .. }))
    );
    assert_eq!(
        machine_eval::evaluate(&function, &[1, 0]),
        Err(MachineEvalError::DivisionByZero)
    );
    assert_eq!(
        machine_eval::evaluate(&function, &[i64::MIN, -1]),
        Err(MachineEvalError::DivisionOverflow)
    );
    for (value, divisor) in [(-9, -2), (9, -2), (-9, 2), (i64::MIN, i64::MIN)] {
        assert_eq!(
            machine_eval::evaluate(&function, &[value, divisor]),
            Ok(value / divisor)
        );
    }
}

#[test]
fn reduced_division_handles_spilled_inputs_and_results() {
    let mut result_graph = Graph::new();
    let start = result_graph.start();
    let dividend = result_graph.parameter(0);
    let four = result_graph.constant(4);
    let quotient = result_graph.binary(BinaryOp::Div, dividend, four);
    let values = (1..20)
        .map(|index| result_graph.parameter(index))
        .collect::<Vec<_>>();
    let mut sum = values[0];
    for value in values.into_iter().skip(1) {
        sum = result_graph.binary(BinaryOp::Add, sum, value);
    }
    let result = result_graph.binary(BinaryOp::Add, sum, quotient);
    result_graph.return_(start, Some(result));
    let spilled_result = native::compile_graph(&result_graph, "spilled_result").unwrap();
    assert!(spilled_result.stack_slots > 0);
    assert_reduced(&spilled_result, 2);
    let final_shift = spilled_result
        .instructions
        .iter()
        .position(|instruction| matches!(instruction, MachineInstr::ShrImm { count: 2, .. }))
        .unwrap();
    assert!(matches!(
        spilled_result.instructions[final_shift + 1],
        MachineInstr::Store { .. }
    ));

    let mut input_graph = Graph::new();
    let start = input_graph.start();
    let values = (0..20)
        .map(|index| input_graph.parameter(index))
        .collect::<Vec<_>>();
    let mut sum = values[1];
    for value in values.iter().skip(2) {
        sum = input_graph.binary(BinaryOp::Add, sum, *value);
    }
    let four = input_graph.constant(4);
    let quotient = input_graph.binary(BinaryOp::Div, values[0], four);
    let result = input_graph.binary(BinaryOp::Add, sum, quotient);
    input_graph.return_(start, Some(result));
    let spilled_input = native::compile_graph(&input_graph, "spilled_input").unwrap();
    assert!(spilled_input.stack_slots > 0);
    assert_reduced(&spilled_input, 2);
    let sign_shift = spilled_input
        .instructions
        .iter()
        .position(|instruction| matches!(instruction, MachineInstr::ShrImm { count: 63, .. }))
        .unwrap();
    assert!(
        spilled_input.instructions[..sign_shift]
            .iter()
            .rev()
            .take(2)
            .any(|instruction| matches!(instruction, MachineInstr::Load { .. }))
    );

    for dividend in VALUES {
        let mut arguments = (1..20).collect::<Vec<_>>();
        arguments.insert(0, dividend);
        let expected = arguments[1..]
            .iter()
            .fold(dividend / 4, |sum, value| sum.wrapping_add(*value));
        for (name, function) in [
            ("spilled result", &spilled_result),
            ("spilled input", &spilled_input),
        ] {
            assert_eq!(
                machine_eval::evaluate(function, &arguments),
                Ok(expected),
                "{name}, dividend={dividend}"
            );
        }
    }
}

#[test]
fn direct_object_emission_accepts_reduced_division() {
    let source = "fn main(value: i64) -> i64 { return value / 4; }";
    let mut graph = frontend::parse(source).unwrap();
    optimizer_pipeline::optimize(&mut graph);
    let graph = placement::place_for_native(&graph).unwrap();
    let object = native_object::compile_graph_to_object(&graph, "main").unwrap();
    assert_eq!(&object[..4], b"\x7fELF");
    assert_eq!(machine_eval::evaluate(&compile(source), &[-9]), Ok(-2));
}
