use kimi_fast_rustc::x86::{MachineFunction, MachineInstr, Reg};
use kimi_fast_rustc::{frontend, machine_eval, native, optimizer_pipeline, placement};

fn compile(source: &str) -> MachineFunction {
    let mut graph = frontend::parse(source).unwrap();
    optimizer_pipeline::optimize(&mut graph);
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn immediate_shifts_use_count_encodings_without_cl() {
    let function = compile("fn main(value: i64) -> i64 { return value << 3; }");
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::ShlImm { count: 3, .. })),
        "{function:#?}"
    );
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::Mov { dst: Reg::Rcx, .. }))
    );
    assert_eq!(machine_eval::evaluate(&function, &[5]), Ok(40));

    let assembly = function.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("shl "));
    assert!(!assembly.contains("cl\n"));
}

#[test]
fn immediate_count_is_masked_to_six_bits() {
    let negative = compile("fn main(value: i64) -> i64 { return value << -1; }");
    assert!(
        negative
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::ShlImm { count: 63, .. }))
    );
    assert_eq!(machine_eval::evaluate(&negative, &[1]), Ok(i64::MIN));

    let large = compile("fn main(value: i64) -> i64 { return value << 65; }");
    assert!(
        large
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::ShlImm { count: 1, .. }))
    );
    assert_eq!(machine_eval::evaluate(&large, &[5]), Ok(10));
}

#[test]
fn immediate_shift_machine_encodings_are_exact() {
    let mut function = MachineFunction::new("immediate_shifts");
    function.instructions = vec![
        MachineInstr::ShlImm {
            dst: Reg::Rax,
            count: 3,
        },
        MachineInstr::ShrImm {
            dst: Reg::Rax,
            count: 4,
        },
        MachineInstr::LShrImm {
            dst: Reg::Rax,
            count: 5,
        },
        MachineInstr::Ret,
    ];
    assert_eq!(
        function.emit_machine_code().unwrap(),
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0xc1, 0xe0, 0x03, 0x48, 0xc1, 0xf8, 0x04, 0x48, 0xc1,
            0xe8, 0x05, 0xc9, 0xc3,
        ]
    );
}
