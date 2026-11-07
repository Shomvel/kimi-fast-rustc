use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, interpreter, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

fn assert_differential(source: &str, cases: &[[i64; 2]]) {
    let graph = frontend::parse(source).unwrap();
    let placed = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&placed, "main").unwrap();
    for arguments in cases {
        let expected = interpreter::interpret(&graph, arguments).unwrap();
        assert_eq!(
            machine_eval::evaluate(&function, arguments),
            Ok(expected),
            "source: {source}; arguments: {arguments:?}"
        );
    }
}

#[test]
fn radix_literals_preserve_signed_and_unsigned_values() {
    let signed =
        compile("fn main() -> i64 { return 0x2a + 0X2A + 0o52 + 0O52 + 0b10_1010 + 0B101010; }");
    assert_eq!(machine_eval::evaluate(&signed, &[]), Ok(252));

    let unsigned =
        compile("fn main() -> u64 { let value: u64 = 0xffff_ffff_ffff_ffff; return value; }");
    assert_eq!(machine_eval::evaluate(&unsigned, &[]), Ok(-1));
}

#[test]
fn isize_is_the_signed_word_type_on_x86_64() {
    let graph = frontend::parse(
        "fn identity(value: isize) -> isize { return value; } fn main() -> isize { return identity(0x2aisize); }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall identity\n"));
    let function = compile("fn main() -> isize { return 0x2aisize; }");
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));

    let minimum = compile("fn main() -> isize { return -0x8000_0000_0000_0000isize; }");
    assert_eq!(machine_eval::evaluate(&minimum, &[]), Ok(i64::MIN));
}

#[test]
fn bitwise_and_shift_compound_assignments_match_the_interpreter() {
    for operator in ["&=", "|=", "^=", "<<=", ">>="] {
        assert_differential(
            &format!(
                "fn main(a: i64, b: i64) -> i64 {{ let mut value = a; value {operator} b; return value; }}"
            ),
            &[[12, 10], [-8, 3], [i64::MIN, 1], [i64::MAX, 65]],
        );
    }
}

#[test]
fn combined_assignments_work_in_unsigned_loop_phi_values() {
    let function = compile(
        "fn main() -> u64 { let mut value = 0x8000_0000_0000_0000u64; value >>= 63; value <<= 6; value |= 2; value ^= 0x68; value += 0; return value; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));

    let source = "fn main() -> i64 { let mut value = 0xff; let mut count = 0; while count < 2 { value &= 0x7f; value ^= 0x20; value <<= 1; value >>= 1; value |= 1; count += 1; } return value; }";
    let function = compile(source);
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(127));
}

#[test]
fn invalid_radix_literals_and_compound_assignments_are_rejected() {
    for source in [
        "fn main() { return 0x; }",
        "fn main() { return 0o8; }",
        "fn main() { return 0b2; }",
        "fn main() { return 0x1i32; }",
        "fn main() { let value = 1; value &= 2; }",
        "fn main() { let mut value = true; value |= false; }",
        "fn main(a: i64, b: u64) -> i64 { let mut value = a; value <<= b; return value; }",
    ] {
        assert!(frontend::parse(source).is_err(), "source: {source}");
    }
}
