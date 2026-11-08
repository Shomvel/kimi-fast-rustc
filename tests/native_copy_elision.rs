use kimi_fast_rustc::x86::{MachineFunction, MachineInstr, Reg};
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn variable_shift_count_does_not_make_an_r11_rcx_round_trip() {
    let function = compile("fn main(value: i64, count: i64) -> i64 { return value << count; }");

    assert!(
        !function.instructions.iter().any(|instruction| matches!(
            instruction,
            MachineInstr::Mov {
                dst: Reg::Rcx,
                src: Reg::R11,
            }
        )),
        "{function:#?}"
    );
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::Shl { dst: Reg::R10 }))
    );

    for (value, count, expected) in [(5, 3, 40), (1, 63, i64::MIN), (7, 64, 7)] {
        assert_eq!(
            machine_eval::evaluate(&function, &[value, count]),
            Ok(expected)
        );
    }
}
