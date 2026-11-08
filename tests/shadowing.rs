use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn let_rhs_uses_the_previous_binding_and_shadowing_can_change_type() {
    let function = compile(
        "fn main(x: i64) -> i64 { let x = x + 1; let x = x * 2; let x = x == 6; if x { return 42; } return 0; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[2]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[3]), Ok(0));
}

#[test]
fn each_shadow_has_its_own_mutability() {
    let function = compile("fn main(x: i64) -> i64 { let mut x = x + 40; x += 1; return x; }");
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(42));

    let error = frontend::parse("fn main(mut x: i64) -> i64 { let x = x + 1; x = 2; return x; }")
        .unwrap_err();
    assert!(error.message.contains("immutable"));
}

#[test]
fn branch_shadows_do_not_leak_into_outer_phis() {
    let function = compile(
        "fn main(value: i64) -> i64 { let mut x = 1; if value > 0 { x = 5; let mut x = x + 1; x = 100; } else { x = 2; let x = 99; } return x; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(5));
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(2));

    let function = compile(
        "fn main(value: i64) -> i64 { let x = 40; if value > 0 { let x = true; } return x + 2; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(42));
}

#[test]
fn loop_shadows_do_not_leak_into_back_edges_or_breaks() {
    let function = compile(
        "fn main() -> i64 { let mut x = 0; let mut i = 0; while i < 4 { x += 1; let mut x = x + 40; x += 100; i += 1; } return x; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(4));

    let function = compile(
        "fn main() -> i64 { let mut x = 0; let mut i = 0; while i < 10 { x += 1; let x = 100; if i == 2 { break; } i += 1; } return x; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(3));
}

#[test]
fn for_variables_shadow_outer_bindings_without_losing_outer_mutability() {
    let function = compile(
        "fn main() -> i64 { let mut x = 40; for x in 0..3 { let x = x + 1; } x += 2; return x; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));

    let function = compile(
        "fn main() -> i64 { let x = 3; let mut sum = 0; for x in 0..x { sum += x; } return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(3));

    let error = frontend::parse("fn main() { let mut x = 0; for x in 0..3 { x = 1; } x = 2; }")
        .unwrap_err();
    assert!(error.message.contains("immutable"));
}

#[test]
fn native_shadowing_example_returns_42() {
    let function = compile(include_str!("../examples/native_shadowing.kfr"));
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}
