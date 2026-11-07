use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn signed_and_unsigned_casts_preserve_bit_patterns() {
    let unsigned = compile("fn main(value: i64) -> u64 { return value as u64; }");
    assert_eq!(machine_eval::evaluate(&unsigned, &[-1]), Ok(-1));
    assert_eq!(machine_eval::evaluate(&unsigned, &[42]), Ok(42));

    let signed = compile("fn main(value: u64) -> i64 { return value as i64; }");
    assert_eq!(machine_eval::evaluate(&signed, &[-1]), Ok(-1));
    assert_eq!(machine_eval::evaluate(&signed, &[42]), Ok(42));
}

#[test]
fn bool_casts_produce_zero_or_one() {
    let signed = compile("fn main(flag: bool) -> i64 { return flag as i64; }");
    let unsigned = compile("fn main(flag: bool) -> usize { return flag as usize; }");
    for (flag, expected) in [(0, 0), (1, 1)] {
        assert_eq!(machine_eval::evaluate(&signed, &[flag]), Ok(expected));
        assert_eq!(machine_eval::evaluate(&unsigned, &[flag]), Ok(expected));
    }
}

#[test]
fn casts_bind_before_arithmetic_and_can_chain() {
    let function = compile(
        "fn main(value: i64) -> i64 { let bits = value as u64 as i64; let flag = bits < 0; return flag as i64 + 41; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[-1]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(41));

    let division = compile("fn main(value: i64) -> u64 { return value as u64 / 2; }");
    assert_eq!(machine_eval::evaluate(&division, &[5]), Ok(2));
}

#[test]
fn casts_work_in_calls_and_ranges() {
    let graph = frontend::parse(
        "fn convert(value: i64) -> u64 { return value as u64; } fn main() -> u64 { let mut total = 0usize; for i in 0..convert(4) { total += i as usize; } return total; }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall convert\n"));
}

#[test]
fn numeric_to_bool_and_numeric_conditions_are_rejected() {
    for source in [
        "fn main() -> bool { return 1 as bool; }",
        "fn main() { if 1 as u64 { let value = 2; } }",
    ] {
        assert!(frontend::parse(source).is_err(), "source: {source}");
    }

    let identity = compile("fn main(flag: bool) -> bool { return flag as bool; }");
    assert_eq!(machine_eval::evaluate(&identity, &[1]), Ok(1));
}
