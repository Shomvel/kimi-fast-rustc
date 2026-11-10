//! Encoded-machine checks for exact division and unchecked arithmetic contracts.
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TypeShape};
use kfrc_son::{
    ArithmeticFlags as F, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType, lower_layout,
    verify_graph,
};
use kfrc_x86::lower_function;
use kfrc_x86::machine::x86::{MachineFunction, MachineInstr};

fn binary(
    symbol: &str,
    width: IntegerWidth,
    signed: bool,
    op: Opcode,
    flags: u32,
    rhs: Option<u64>,
) -> MachineFunction {
    let bits = width.bits(64);
    let mut types = RecordedTypes::new();
    let ty = types.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign {
            size: u64::from(bits / 8),
            align: u64::from(bits / 8),
        }),
    );
    let arity = if rhs.is_some() { 1 } else { 2 };
    let mut graph = FunctionGraph::new(
        symbol,
        FunctionAbi {
            args: vec![PassMode::Direct; arity],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Specialized;
    let mut outputs = vec![ValueType::Control, ValueType::Effect];
    outputs.extend((0..arity).map(|_| ValueType::Rust(ty)));
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(outputs),
    );
    graph.start = start;
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let lhs = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        2,
        ValueType::Rust(ty),
    );
    let rhs = if let Some(value) = rhs {
        graph.add_constant(u128::from(value), ty)
    } else {
        graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            3,
            ValueType::Rust(ty),
        )
    };
    let result = graph.add_node(
        op,
        if op.is_fixed() { control } else { NodeId::NONE },
        &[lhs, rhs],
        flags,
        ValueType::Rust(ty),
    );
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, result],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    lower_function(&graph).unwrap()
}

fn divides(machine: &MachineFunction) -> usize {
    machine
        .instructions
        .iter()
        .filter(|op| matches!(op, MachineInstr::IDiv { .. } | MachineInstr::UDiv { .. }))
        .count()
}

#[test]
fn exact_constants_select_multiply_and_shifts_but_plain_division_remains() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        for signed in [false, true] {
            let op = if signed {
                Opcode::DivSigned
            } else {
                Opcode::DivUnsigned
            };
            for divisor in [1, 2, 3, 24, 127] {
                let exact = binary("exact", width, signed, op, F::EXACT, Some(divisor));
                assert_eq!(divides(&exact), 0, "{width:?} signed={signed} /{divisor}");
                exact.emit_machine_code().unwrap();
            }
            assert_eq!(divides(&binary("plain", width, signed, op, 0, Some(3))), 1);
        }
    }
    // Unhandled negative signed divisor retains a real divide; no unsigned
    // reinterpretation of its bits is permitted by the exactness contract.
    assert_eq!(
        divides(&binary(
            "negative",
            IntegerWidth::W64,
            true,
            Opcode::DivSigned,
            F::EXACT,
            Some((-3i64) as u64)
        )),
        1
    );
}

#[test]
fn unchecked_contracts_remove_only_the_result_or_shift_amount_mask() {
    // Width canonicalization is a mask or a zero/sign extension.
    let ands = |machine: MachineFunction| {
        machine
            .instructions
            .iter()
            .filter(|op| matches!(op, MachineInstr::And { .. } | MachineInstr::Extend { .. }))
            .count()
    };
    let ordinary = ands(binary(
        "add",
        IntegerWidth::W32,
        false,
        Opcode::Add,
        0,
        None,
    ));
    let unchecked = ands(binary(
        "add_nuw",
        IntegerWidth::W32,
        false,
        Opcode::Add,
        F::NUW,
        None,
    ));
    assert_eq!(ordinary, unchecked + 1);
    assert!(
        unchecked >= 2,
        "argument upper bits still need ABI canonicalization"
    );
    let ordinary = ands(binary(
        "shift",
        IntegerWidth::W32,
        false,
        Opcode::ShiftLeft,
        0,
        None,
    ));
    let unchecked = ands(binary(
        "shift_in_range",
        IntegerWidth::W32,
        false,
        Opcode::ShiftLeft,
        F::AMOUNT_IN_RANGE,
        None,
    ));
    assert_eq!(ordinary, unchecked + 1);
}

#[test]
fn constant_shifts_keep_immediates_with_width_specific_wrapping() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let bits = u64::from(width.bits(64));
        for (op, signed) in [
            (Opcode::ShiftLeft, false),
            (Opcode::ShiftRightLogical, false),
            (Opcode::ShiftRightArithmetic, true),
        ] {
            for count in [1, bits - 1, bits + 1, 255] {
                let machine = binary("constant_shift", width, signed, op, 0, Some(count));
                assert!(
                    !machine.instructions.iter().any(|instruction| matches!(
                        instruction,
                        MachineInstr::Shl { .. }
                            | MachineInstr::Shr { .. }
                            | MachineInstr::LShr { .. }
                    )),
                    "constant shift used CL: {width:?} {op:?} {count}"
                );
                let expected = (count & (bits - 1)) as u8;
                assert!(
                    machine
                        .instructions
                        .iter()
                        .any(|instruction| match (op, instruction) {
                            (Opcode::ShiftLeft, MachineInstr::ShlImm { count, .. })
                            | (Opcode::ShiftRightArithmetic, MachineInstr::ShrImm { count, .. })
                            | (Opcode::ShiftRightLogical, MachineInstr::LShrImm { count, .. }) =>
                                *count == expected,
                            _ => false,
                        }),
                    "missing immediate shift: {width:?} {op:?} {count}"
                );
                machine.emit_machine_code().unwrap();
            }
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn native_exact_division_matches_defined_signed_and_unsigned_inputs() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::{fs, process::Command};
    let directory =
        std::env::temp_dir().join(format!("kfrc-exact-contract-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let mut functions = Vec::new();
    let mut declarations = String::from("#include <stdint.h>\n");
    let mut checks = String::from("int main(void) {\n");
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let bits = width.bits(64);
        for signed in [false, true] {
            let op = if signed {
                Opcode::DivSigned
            } else {
                Opcode::DivUnsigned
            };
            for divisor in [1u64, 2, 3, 24, 127] {
                let name = format!("exact_{}", functions.len());
                let ctype = if signed { "int64_t" } else { "uint64_t" };
                declarations.push_str(&format!("extern {ctype} {name}({ctype});\n"));
                functions.push(FunctionCode {
                    function: binary(&name, width, signed, op, F::EXACT, Some(divisor)),
                    binding: SymbolBinding::Global { hidden: false },
                });
                let (minimum, maximum) = if signed {
                    (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
                } else {
                    (0, (1i128 << bits) - 1)
                };
                for quotient in [
                    minimum / i128::from(divisor),
                    -127,
                    -7,
                    -1,
                    0,
                    1,
                    7,
                    127,
                    maximum / i128::from(divisor),
                ] {
                    let numerator = quotient * i128::from(divisor);
                    if !(minimum..=maximum).contains(&numerator) {
                        continue;
                    }
                    checks.push_str(&format!("if ({name}(({ctype})UINT64_C(0x{:x})) != ({ctype})UINT64_C(0x{:x})) return 1;\n", numerator as u64, quotient as u64));
                }
            }
        }
    }
    checks.push_str("return 0;\n}\n");
    let object = directory.join("exact.o");
    let source = directory.join("exact.c");
    let executable = directory.join("exact");
    fs::write(&object, write_object(&functions).unwrap()).unwrap();
    fs::write(&source, declarations + &checks).unwrap();
    let compiled = Command::new("cc")
        .args(["-O2", "-no-pie"])
        .arg(&source)
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new(&executable).output().unwrap();
    assert!(
        executed.status.success(),
        "native exact quotient mismatch: {executed:?}"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn native_constant_shifts_match_wrapping_integer_semantics() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::{fs, process::Command};
    let directory =
        std::env::temp_dir().join(format!("kfrc-immediate-shift-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let mut functions = Vec::new();
    let mut source = String::from("#include <stdint.h>\n");
    let mut checks = String::from("int main(void) {\n");
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let bits = width.bits(64);
        let mask = (1u128 << bits) - 1;
        for (op, signed) in [
            (Opcode::ShiftLeft, false),
            (Opcode::ShiftRightLogical, false),
            (Opcode::ShiftRightArithmetic, true),
        ] {
            for count in [0, 1, bits - 1, bits, bits + 1, 255] {
                let name = format!("shift_{}", functions.len());
                source.push_str(&format!("extern uint64_t {name}(uint64_t);\n"));
                functions.push(FunctionCode {
                    function: binary(&name, width, signed, op, 0, Some(u64::from(count))),
                    binding: SymbolBinding::Global { hidden: false },
                });
                for value in [
                    0,
                    1,
                    mask,
                    1u128 << (bits - 1),
                    0x0123_4567_89ab_cdef & mask,
                ] {
                    let amount = count & (bits - 1);
                    let signed_value = (value << (128 - bits)) as i128 >> (128 - bits);
                    let expected = match op {
                        Opcode::ShiftLeft => ((value << amount) & mask) as u64,
                        Opcode::ShiftRightLogical => (value >> amount) as u64,
                        Opcode::ShiftRightArithmetic => (signed_value >> amount) as u64,
                        _ => unreachable!(),
                    };
                    checks.push_str(&format!(
                        "if ({name}(UINT64_C(0x{value:x})) != UINT64_C(0x{expected:x})) return 1;\n"
                    ));
                }
            }
        }
    }
    checks.push_str("return 0;\n}\n");
    let object = directory.join("shifts.o");
    let harness = directory.join("shifts.c");
    let executable = directory.join("shifts");
    fs::write(&object, write_object(&functions).unwrap()).unwrap();
    fs::write(&harness, source + &checks).unwrap();
    let compiled = Command::new("cc")
        .args(["-O2", "-no-pie"])
        .arg(&harness)
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new(&executable).output().unwrap();
    assert!(
        executed.status.success(),
        "native wrapping shift mismatch: {executed:?}"
    );
    fs::remove_dir_all(directory).unwrap();
}
