use kimi_fast_rustc::{frontend, native_program, placement};

const SOURCE: &str = "fn answer() -> i64 { return 42; }\nfn main() -> i64 { return answer(); }";

#[test]
fn zero_argument_calls_emit_symbols_and_relocations() {
    let graph = frontend::parse(SOURCE).unwrap();
    assert_eq!(graph.functions.len(), 2);
    let graph = placement::place_for_native(&graph).unwrap();
    let program = native_program::compile_graph(&graph).unwrap();
    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tcall answer\n"));
    assert!(assembly.contains("\tmov rcx, rax\n"));
    assert!(!assembly.contains("llvm"));

    let object = program.emit_object().unwrap();
    assert_eq!(&object[..4], b"\x7fELF");
    assert!(
        object
            .windows(b"answer".len())
            .any(|window| window == b"answer")
    );
    assert!(
        object
            .windows(b".rela.text".len())
            .any(|window| window == b".rela.text")
    );
}

#[test]
fn register_arguments_are_loaded_before_the_call() {
    let graph =
        frontend::parse("fn id(x: i64) -> i64 { return x; } fn main() -> i64 { return id(42); }")
            .unwrap();
    let program = native_program::compile_graph(&graph).unwrap();
    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tmovabs rcx, 0x2a\n"));
    assert!(assembly.contains("\tmov rdi, QWORD PTR [rbp"));
    assert!(assembly.contains("\tcall id\n"));
}

#[test]
fn call_arity_is_checked() {
    let error =
        frontend::parse("fn id(x: i64) -> i64 { return x; } fn main() -> i64 { return id(); }")
            .unwrap_err();
    assert!(error.message.contains("expects 1 arguments, found 0"));
}

#[test]
fn unknown_calls_are_rejected_during_parsing() {
    assert!(frontend::parse("fn main() -> i64 { return missing(); }").is_err());
}

#[test]
fn stack_arguments_are_stored_before_the_call() {
    let graph = frontend::parse(
        "fn sum8(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64) -> i64 { return a + b + c + d + e + f + g + h; } fn main() -> i64 { return sum8(1, 2, 3, 4, 5, 6, 7, 8); }",
    )
    .unwrap();
    let program = native_program::compile_graph(&graph).unwrap();
    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("QWORD PTR [rsp+0]"));
    assert!(assembly.contains("QWORD PTR [rsp+8]"));
    assert!(assembly.contains("\tcall sum8\n"));
}
