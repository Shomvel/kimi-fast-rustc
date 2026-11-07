use kimi_fast_rustc::{frontend, machine_eval, native, native_object, placement};

fn compile(source: &str) -> kimi_fast_rustc::x86::MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn if_else_selects_the_expected_branch() {
    let function =
        compile("fn main(value: i64) -> i64 { return if value > 0 { 40 + 2 } else { 0 - 7 }; }");
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(-7));
    assert_eq!(machine_eval::evaluate(&function, &[-9]), Ok(-7));

    let assembly = function.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tje "));
    assert!(assembly.contains("\tjmp"));
    assert!(!assembly.contains("llvm"));
}

#[test]
fn nested_if_preserves_control_and_phi_values() {
    let function = compile(
        "fn main(value: i64) -> i64 { return if value > 10 { if value > 20 { 1 } else { 2 } } else { 3 }; }",
    );
    for (argument, expected) in [(30, 1), (20, 2), (15, 2), (10, 3), (0, 3), (-4, 3)] {
        assert_eq!(machine_eval::evaluate(&function, &[argument]), Ok(expected));
    }
}

#[test]
fn direct_objects_resolve_control_relocations() {
    let graph =
        frontend::parse("fn main(value: i64) -> i64 { return if value != 0 { 1 } else { 0 }; }")
            .unwrap();
    let object = native_object::compile_graph_to_object(&graph, "main").unwrap();
    assert_eq!(&object[..4], b"\x7fELF");
    assert!(object.windows(2).any(|window| window == [0x0f, 0x84]));
    assert!(object.windows(1).any(|window| window == [0xe9]));
}
