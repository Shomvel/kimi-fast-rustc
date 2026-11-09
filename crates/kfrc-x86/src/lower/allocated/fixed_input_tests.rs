use super::*;
use crate::machine::eval;
use kfrc_body::{FunctionAbi, RecordedTypes};

fn v(index: u32) -> VirtualValue {
    VirtualValue::Reg(VirtualReg(index))
}
fn scalar(instruction: VirtualInstr) -> LowerInstr {
    LowerInstr::Scalar(instruction)
}
fn compile_stream(stream: Vec<LowerInstr>, count: u32) -> MachineFunction {
    let graph = FunctionGraph::new(
        "fixed_input",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    compile(&graph, stream, count, &[]).unwrap()
}
fn args(count: u32) -> Vec<LowerInstr> {
    (0..count)
        .map(|index| {
            scalar(VirtualInstr::LoadArg {
                dst: VirtualReg(index),
                index: index as usize,
            })
        })
        .collect()
}

#[test]
fn unrelated_live_values_across_repeated_cl_shifts_need_no_save_slots() {
    let mut code = args(3);
    let mut value = v(0);
    for index in 3..35 {
        code.push(scalar(VirtualInstr::LShr {
            dst: VirtualReg(index),
            lhs: value,
            rhs: v(2),
        }));
        value = v(index);
    }
    code.push(scalar(VirtualInstr::Add {
        dst: VirtualReg(35),
        lhs: value,
        rhs: v(0),
    }));
    code.push(scalar(VirtualInstr::Add {
        dst: VirtualReg(36),
        lhs: v(35),
        rhs: v(1),
    }));
    code.push(scalar(VirtualInstr::Return(v(36))));
    let function = compile_stream(code, 37);
    assert_eq!(function.stack_slots, 0, "{:?}", function.instructions);
    for input in [-91i64, 0, 100, i64::MAX] {
        for count in [0i64, 1, 7, 63] {
            let shifted = (0..32).fold(input as u64, |value, _| value.wrapping_shr(count as u32));
            let expected = (shifted as i64).wrapping_add(input).wrapping_add(17);
            assert_eq!(
                eval::evaluate(&function, &[input, 17, count]).unwrap(),
                expected
            );
        }
    }
}

#[test]
fn same_value_can_supply_both_shift_operands_and_remain_live() {
    let mut code = args(1);
    code.extend([
        scalar(VirtualInstr::Shl {
            dst: VirtualReg(1),
            lhs: v(0),
            rhs: v(0),
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(2),
            lhs: v(1),
            rhs: v(0),
        }),
        scalar(VirtualInstr::Return(v(2))),
    ]);
    let function = compile_stream(code, 3);
    assert_eq!(function.stack_slots, 0, "{:?}", function.instructions);
    for value in [-91i64, 0, 1, 7, 63, 100] {
        assert_eq!(
            eval::evaluate(&function, &[value]).unwrap(),
            value.wrapping_shl(value as u32).wrapping_add(value)
        );
    }
}

#[test]
fn fixed_input_pins_do_not_exclude_new_division_or_call_results() {
    let mut divide = args(2);
    divide.extend([
        scalar(VirtualInstr::Div {
            dst: VirtualReg(2),
            lhs: v(0),
            rhs: v(1),
        }),
        scalar(VirtualInstr::Return(v(2))),
    ]);
    let selected = constraints::select(divide.clone());
    let allocation = allocation::Allocation::new(&selected, 3).unwrap();
    let position = selected
        .iter()
        .position(|i| matches!(i.operation, Operation::Divide { .. }))
        .unwrap();
    assert_eq!(
        allocation.home(VirtualReg(2), position).unwrap(),
        Location::Reg(Reg::Rax)
    );
    let function = compile_stream(divide, 3);
    for (a, b) in [(-101, 7), (101, 7), (1, 1)] {
        assert_eq!(eval::evaluate(&function, &[a, b]).unwrap(), a / b);
    }

    let mut call = args(3);
    call.extend([
        LowerInstr::CallPair {
            first: VirtualReg(3),
            second: VirtualReg(4),
            target: CallDestination::Direct("callee_pair".into()),
            args: vec![v(0), v(1), v(2)],
        },
        LowerInstr::ReturnPair {
            first: v(3),
            second: v(4),
        },
    ]);
    let selected = constraints::select(call.clone());
    let allocation = allocation::Allocation::new(&selected, 5).unwrap();
    let position = selected
        .iter()
        .position(|i| matches!(i.operation, Operation::Call(_)))
        .unwrap();
    assert_eq!(
        allocation.home(VirtualReg(3), position).unwrap(),
        Location::Reg(Reg::Rax)
    );
    assert!(matches!(
        allocation.home(VirtualReg(4), position).unwrap(),
        Location::Reg(_)
    ));
    let function = compile_stream(call, 5); // The checker verifies fixed RAX/RDX outputs after clobbers.
    assert!(
        function
            .instructions
            .iter()
            .any(|i| matches!(i, MachineInstr::Call { .. }))
    );
}

#[test]
fn input_pin_queries_distinguish_owner_live_through_dead_and_new_values() {
    let selected = constraints::select(vec![
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(0),
            index: 0,
        }),
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(1),
            index: 1,
        }),
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(2),
            index: 2,
        }),
        scalar(VirtualInstr::Shl {
            dst: VirtualReg(3),
            lhs: v(0),
            rhs: v(1),
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(4),
            lhs: v(3),
            rhs: v(2),
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(5),
            lhs: v(4),
            rhs: v(1),
        }),
        scalar(VirtualInstr::Return(v(5))),
    ]);
    let facts: Vec<_> = selected.iter().map(Instruction::facts).collect();
    let live = liveness::analyze(&facts);
    let position = selected
        .iter()
        .position(|i| matches!(i.operation, Operation::Shift(_, _)))
        .unwrap();
    let pins = [Pin {
        position,
        kind: PinKind::Input(v(1)),
    }];
    for (value, expected) in [
        (0, usize::MAX),
        (1, usize::MAX),
        (2, position),
        (3, usize::MAX),
    ] {
        assert_eq!(
            next_blocking_pin(
                &pins,
                VirtualReg(value),
                0,
                selected.len(),
                0,
                &selected,
                &live
            ),
            expected,
            "v{value}"
        );
    }
    // Queries after a position skip all typed pins at that position.
    assert_eq!(
        next_blocking_pin(
            &pins,
            VirtualReg(2),
            position + 1,
            selected.len(),
            0,
            &selected,
            &live
        ),
        usize::MAX
    );
}
