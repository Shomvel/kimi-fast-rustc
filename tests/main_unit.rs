use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn evaluate(source: &str) -> i64 {
    let graph = frontend::parse(source).unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&graph, "main").unwrap();
    machine_eval::evaluate(&function, &[]).unwrap()
}

#[test]
fn standard_main_forms_return_zero() {
    assert_eq!(evaluate("fn main() {}"), 0);
    assert_eq!(evaluate("fn main() { return; }"), 0);
    assert_eq!(evaluate("fn main() { let value = 1; }"), 0);
}

#[test]
fn i64_main_still_requires_a_value() {
    assert!(frontend::parse("fn main() -> i64 { return; }").is_err());
    assert!(frontend::parse("fn main() -> i64 {}").is_err());
}
