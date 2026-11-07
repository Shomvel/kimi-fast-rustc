use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn inclusive_ranges_include_the_end_and_skip_empty_ranges() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; for i in 0..=3 { sum += i; } for i in 3..=0 { sum += 100; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(6));
}

#[test]
fn continue_on_the_last_inclusive_iteration_exits_cleanly() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; for i in 0..=5 { if i % 2 == 1 { continue; } sum += i; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(6));
}

#[test]
fn inclusive_u64_max_does_not_wrap_to_zero() {
    let singleton = compile(
        "fn main() -> u64 { let mut total = 0u64; for i in 18446744073709551615u64..=18446744073709551615u64 { total += i; } return total; }",
    );
    assert_eq!(machine_eval::evaluate(&singleton, &[]), Ok(-1));

    let pair = compile(
        "fn main() -> u64 { let mut count = 0u64; for i in 18446744073709551614u64..=18446744073709551615u64 { count += 1; } return count; }",
    );
    assert_eq!(machine_eval::evaluate(&pair, &[]), Ok(2));
}

#[test]
fn inclusive_ranges_support_early_return() {
    let function =
        compile("fn main() -> i64 { for i in 0..=10 { if i == 3 { return i + 39; } } return 0; }");
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}
