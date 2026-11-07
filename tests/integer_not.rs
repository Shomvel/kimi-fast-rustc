use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn signed_integer_not_matches_bitwise_complement() {
    let function = compile("fn main(value: i64) -> i64 { return !value; }");
    for value in [i64::MIN, -43, -1, 0, 1, 42, i64::MAX] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(!value));
    }
}

#[test]
fn unsigned_integer_not_preserves_the_full_bit_pattern() {
    let function = compile("fn main(value: u64) -> u64 { return !value; }");
    for value in [i64::MIN, -1, 0, 42, i64::MAX] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(!value));
    }
}

#[test]
fn not_precedence_double_inversion_and_loop_phis_are_preserved() {
    let precedence = compile("fn main(value: isize) -> isize { return !!value & 0xff; }");
    for value in [-1, 0, 42, 256] {
        assert_eq!(
            machine_eval::evaluate(&precedence, &[value]),
            Ok(value & 0xff)
        );
    }

    let loops = compile(
        "fn main() -> i64 { let mut value = 0; let mut count = 0; while count < 3 { value = !value; count += 1; } return if value == -1 { 42 } else { 0 }; }",
    );
    assert_eq!(machine_eval::evaluate(&loops, &[]), Ok(42));
}

#[test]
fn integer_not_emits_native_code_and_rejects_unit_or_signed_overflow() {
    let graph = frontend::parse("fn main() -> i64 { return !(-43); }").unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tnot "));
    assert_eq!(
        &native_program::compile_graph_to_object(&graph).unwrap()[..4],
        b"\x7fELF"
    );

    for source in [
        "fn helper() {} fn main() { helper(); return !helper(); }",
        "fn main() -> i64 { return !9223372036854775808; }",
    ] {
        assert!(frontend::parse(source).is_err(), "source: {source}");
    }
}
