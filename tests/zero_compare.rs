use kimi_fast_rustc::son::{CompareOp, Graph};
use kimi_fast_rustc::x86::{Condition, MachineFunction, MachineInstr, Operand, Reg, StackSlot};
use kimi_fast_rustc::{frontend, machine_eval, native, placement};

const VALUES: [i64; 5] = [i64::MIN, -1, 0, 1, i64::MAX];

fn function(instructions: Vec<MachineInstr>) -> MachineFunction {
    let mut function = MachineFunction::new("zero_compare");
    function.instructions = instructions;
    function
}

fn comparisons() -> [(CompareOp, Condition); 10] {
    [
        (CompareOp::Eq, Condition::Eq),
        (CompareOp::Ne, Condition::NotEq),
        (CompareOp::Lt, Condition::SignedLess),
        (CompareOp::Le, Condition::SignedLessEq),
        (CompareOp::Gt, Condition::SignedGreater),
        (CompareOp::Ge, Condition::SignedGreaterEq),
        (CompareOp::ULt, Condition::UnsignedBelow),
        (CompareOp::ULe, Condition::UnsignedBelowEq),
        (CompareOp::UGt, Condition::UnsignedAbove),
        (CompareOp::UGe, Condition::UnsignedAboveEq),
    ]
}

fn zero_condition(condition: Condition, value: i64) -> i64 {
    i64::from(match condition {
        Condition::Eq => value == 0,
        Condition::NotEq => value != 0,
        Condition::SignedLess => value < 0,
        Condition::SignedLessEq => value <= 0,
        Condition::SignedGreater => value > 0,
        Condition::SignedGreaterEq => value >= 0,
        Condition::UnsignedBelow => false,
        Condition::UnsignedBelowEq => value == 0,
        Condition::UnsignedAbove => value != 0,
        Condition::UnsignedAboveEq => true,
    })
}

#[test]
fn test_and_cmp_zero_have_exact_register_encodings() {
    for (reg, rex, modrm) in [(Reg::Rax, 0x48, 0xc0), (Reg::R11, 0x4d, 0xdb)] {
        let expected = vec![0x55, 0x48, 0x89, 0xe5, rex, 0x85, modrm, 0xc9, 0xc3];
        for instruction in [
            MachineInstr::Test { reg },
            MachineInstr::Cmp {
                lhs: reg,
                rhs: Operand::Imm(0),
            },
        ] {
            let function = function(vec![instruction, MachineInstr::Ret]);
            assert_eq!(
                function.emit_machine_code().unwrap(),
                expected,
                "{instruction:?}"
            );
            let assembly = function.emit_gnu_assembly().unwrap();
            assert!(
                assembly.contains(&format!("\ttest {reg}, {reg}\n")),
                "{assembly}"
            );
            assert!(!assembly.contains("\tcmp "), "{assembly}");
        }
    }
}

#[test]
fn test_zero_preserves_every_setcc_condition_and_opcode() {
    for (_, condition) in comparisons() {
        let opcode = match condition {
            Condition::Eq => 0x94,
            Condition::NotEq => 0x95,
            Condition::SignedLess => 0x9c,
            Condition::SignedLessEq => 0x9e,
            Condition::SignedGreater => 0x9f,
            Condition::SignedGreaterEq => 0x9d,
            Condition::UnsignedBelow => 0x92,
            Condition::UnsignedBelowEq => 0x96,
            Condition::UnsignedAbove => 0x97,
            Condition::UnsignedAboveEq => 0x93,
        };
        let function = function(vec![
            MachineInstr::LoadArg {
                dst: Reg::Rax,
                index: 0,
            },
            MachineInstr::Test { reg: Reg::Rax },
            MachineInstr::SetCc {
                condition,
                dst: Reg::Rax,
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(
            function.emit_machine_code().unwrap(),
            [
                0x55, 0x48, 0x89, 0xe5, 0x48, 0x89, 0xf8, 0x48, 0x85, 0xc0, 0x0f, opcode, 0xc0,
                0x0f, 0xb6, 0xc0, 0xc9, 0xc3,
            ],
            "{condition:?}"
        );
        for value in VALUES {
            assert_eq!(
                machine_eval::evaluate(&function, &[value]),
                Ok(zero_condition(condition, value)),
                "{condition:?}, value {value}"
            );
        }
    }
}

#[test]
fn native_compare_and_direct_branch_use_test_for_zero() {
    for (operation, condition) in comparisons() {
        let mut compare_graph = Graph::new();
        let start = compare_graph.start();
        let value = compare_graph.parameter(0);
        let zero = compare_graph.constant(0);
        let comparison = compare_graph.compare(operation, value, zero);
        compare_graph.return_(start, Some(comparison));
        let compare = native::compile_graph(&compare_graph, "compare_zero").unwrap();
        assert!(
            compare.instructions.windows(2).any(|instructions| matches!(
                instructions,
                [
                    MachineInstr::Test { .. },
                    MachineInstr::SetCc {
                        condition: actual,
                        ..
                    }
                ] if *actual == condition
            )),
            "{operation:?}: {:?}",
            compare.instructions
        );
        assert!(compare.emit_gnu_assembly().unwrap().contains("\ttest "));
        for value in VALUES {
            assert_eq!(
                machine_eval::evaluate(&compare, &[value]),
                Ok(zero_condition(condition, value)),
                "Compare {operation:?}, value {value}"
            );
        }

        let (ty, operator) = match operation {
            CompareOp::Eq => ("i64", "=="),
            CompareOp::Ne => ("i64", "!="),
            CompareOp::Lt => ("i64", "<"),
            CompareOp::Le => ("i64", "<="),
            CompareOp::Gt => ("i64", ">"),
            CompareOp::Ge => ("i64", ">="),
            CompareOp::ULt => ("u64", "<"),
            CompareOp::ULe => ("u64", "<="),
            CompareOp::UGt => ("u64", ">"),
            CompareOp::UGe => ("u64", ">="),
        };
        let branch_graph = frontend::parse(&format!(
            "fn main(value: {ty}) -> i64 {{ return if value {operator} 0 {{ 42 }} else {{ 0 }}; }}"
        ))
        .unwrap();
        let branch_graph = placement::place_for_native(&branch_graph).unwrap();
        let branch = native::compile_graph(&branch_graph, "branch_zero").unwrap();
        assert!(
            branch.instructions.windows(2).any(|instructions| matches!(
                instructions,
                [MachineInstr::Test { .. }, MachineInstr::Jcc { .. }]
            )),
            "{operation:?}: {:?}",
            branch.instructions
        );
        assert!(
            !branch
                .instructions
                .iter()
                .any(|instruction| matches!(instruction, MachineInstr::SetCc { .. }))
        );
        assert!(branch.emit_gnu_assembly().unwrap().contains("\ttest "));
        for value in VALUES {
            assert_eq!(
                machine_eval::evaluate(&branch, &[value]),
                Ok(42 * zero_condition(condition, value)),
                "Test {operation:?}, value {value}: {:?}",
                branch.instructions
            );
        }
    }
}

#[test]
fn spilled_cmp_rhs_keeps_cmp_encoding_and_semantics() {
    let mut function = function(vec![
        MachineInstr::MovImm {
            dst: Reg::Rcx,
            value: -1,
        },
        MachineInstr::Store {
            slot: StackSlot(0),
            src: Reg::Rcx,
        },
        MachineInstr::MovImm {
            dst: Reg::Rax,
            value: 0,
        },
        MachineInstr::Cmp {
            lhs: Reg::Rax,
            rhs: Operand::Stack(StackSlot(0)),
        },
        MachineInstr::SetCc {
            condition: Condition::SignedGreater,
            dst: Reg::Rax,
        },
        MachineInstr::Ret,
    ]);
    function.stack_slots = 1;
    let code = function.emit_machine_code().unwrap();
    assert!(
        code.windows(4)
            .any(|bytes| bytes == [0x48, 0x3b, 0x45, 0xf8])
    );
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::Test { .. }))
    );
    let assembly = function.emit_gnu_assembly().unwrap();
    assert!(
        assembly.contains("\tcmp rax, QWORD PTR [rbp-8]\n"),
        "{assembly}"
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(1));
}

#[test]
fn native_zero_compare_example_evaluates_to_42() {
    let graph = frontend::parse(
        "fn main(value: i64) -> i64 { let nonnegative = value >= 0; return if value == 0 { 40 + nonnegative as i64 + 1 } else { 0 }; }",
    )
    .unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&graph, "main").unwrap();
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::Test { .. }))
    );
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[-1]), Ok(0));
}
