//! Encoded-machine checks for constant division, shifts and arithmetic contracts.
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
    compile(binary_graph(symbol, width, signed, op, flags, rhs))
}

fn compile(mut graph: FunctionGraph) -> MachineFunction {
    verify_graph(&mut graph).unwrap();
    lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    lower_function(&graph).unwrap()
}

fn binary_graph(
    symbol: &str,
    width: IntegerWidth,
    signed: bool,
    op: Opcode,
    flags: u32,
    rhs: Option<u64>,
) -> FunctionGraph {
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
    graph
}

#[test]
fn quotient_and_remainder_share_one_reciprocal_product() {
    use kfrc_x86::machine::eval;
    for signed in [false, true] {
        let div = if signed {
            Opcode::DivSigned
        } else {
            Opcode::DivUnsigned
        };
        let rem = if signed {
            Opcode::RemSigned
        } else {
            Opcode::RemUnsigned
        };
        for first_remainder in [false, true] {
            let mut graph = binary_graph(
                "div_rem",
                IntegerWidth::W64,
                signed,
                if first_remainder { rem } else { div },
                0,
                Some(7),
            );
            let ret = graph
                .ids()
                .find(|&n| graph.op(n) == Opcode::Return)
                .unwrap();
            let first = graph.input(ret, 1);
            let operands = graph.inputs(first).to_vec();
            let ty = graph.value_type(first).clone();
            let second = graph.add_node(
                if first_remainder { div } else { rem },
                graph.control(first),
                &operands,
                0,
                ty.clone(),
            );
            let sum = graph.add_node(Opcode::Add, NodeId::NONE, &[first, second], 0, ty);
            graph.set_input(ret, 1, sum);
            let machine = compile(graph);
            assert_eq!(
                machine
                    .instructions
                    .iter()
                    .filter(|i| matches!(i, MachineInstr::MulHigh { .. }))
                    .count(),
                1
            );
            for input in [0i64, 1, 6, 7, 8, -1, -8, i64::MIN, i64::MAX] {
                let expected = if signed {
                    (input / 7).wrapping_add(input % 7)
                } else {
                    ((input as u64 / 7).wrapping_add(input as u64 % 7)) as i64
                };
                assert_eq!(eval::evaluate(&machine, &[input]).unwrap(), expected);
            }
        }
    }
}

#[test]
fn quotient_is_not_reused_from_a_sibling_branch() {
    let mut g = binary_graph(
        "branch_division",
        IntegerWidth::W64,
        false,
        Opcode::DivUnsigned,
        0,
        Some(7),
    );
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let first = g.input(ret, 1);
    let lhs = g.input(first, 0);
    let rhs = g.input(first, 1);
    let ty = g.value_type(first).clone();
    let boolean = g
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let predicate = g.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[lhs, rhs],
        kfrc_son::ComparePredicate::UnsignedLess as u32,
        ValueType::Rust(boolean),
    );
    let branch = g.add_node(
        Opcode::If,
        g.control(ret),
        &[predicate],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let yes = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        0,
        ValueType::Control,
    );
    let no = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        1,
        ValueType::Control,
    );
    g.set_control(first, yes);
    g.set_control(ret, yes);
    let remainder = g.add_node(Opcode::RemUnsigned, no, &[lhs, rhs], 0, ty);
    g.add_node(
        Opcode::Return,
        no,
        &[g.input(ret, 0), remainder],
        0,
        ValueType::Control,
    );
    let machine = compile(g);
    assert_eq!(
        machine
            .instructions
            .iter()
            .filter(|i| matches!(i, MachineInstr::MulHigh { .. }))
            .count(),
        2
    );
    for n in [0u64, 1, 6, 7, 8, 35, u64::MAX] {
        assert_eq!(
            kfrc_x86::machine::eval::evaluate(&machine, &[n as i64]).unwrap(),
            (if n < 7 { n / 7 } else { n % 7 }) as i64
        );
    }
}

fn divides(machine: &MachineFunction) -> usize {
    machine
        .instructions
        .iter()
        .filter(|op| matches!(op, MachineInstr::IDiv { .. } | MachineInstr::UDiv { .. }))
        .count()
}

#[test]
fn exact_and_plain_constants_select_multiply_and_shifts() {
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
            assert_eq!(divides(&binary("plain", width, signed, op, 0, Some(3))), 0);
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
fn ordinary_constant_quotients_and_remainders_match_rust() {
    use kfrc_x86::machine::eval;
    let mut seed = 0x13579bdf2468ace0u64;
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let bits = width.bits(64);
        let mask = u64::MAX >> (64 - bits);
        for signed in [false, true] {
            let max_divisor = if signed { mask >> 1 } else { mask };
            let mut divisors = vec![
                1,
                2,
                3,
                7,
                10,
                31,
                60,
                127,
                256,
                3600,
                86400,
                max_divisor,
                max_divisor - 1,
            ];
            if bits == 8 {
                divisors.extend(1..=max_divisor);
            }
            divisors.retain(|d| *d <= max_divisor);
            divisors.sort_unstable();
            divisors.dedup();
            for divisor in divisors {
                for remainder in [false, true] {
                    let op = match (signed, remainder) {
                        (true, false) => Opcode::DivSigned,
                        (true, true) => Opcode::RemSigned,
                        (false, false) => Opcode::DivUnsigned,
                        (false, true) => Opcode::RemUnsigned,
                    };
                    let machine = binary("constant_division", width, signed, op, 0, Some(divisor));
                    assert_eq!(divides(&machine), 0);
                    machine.emit_machine_code().unwrap();
                    let mut inputs = vec![
                        0,
                        1,
                        divisor - 1,
                        divisor,
                        divisor.wrapping_add(1),
                        mask,
                        mask - 1,
                        1u64 << (bits - 1),
                    ];
                    if bits == 8 {
                        inputs.extend(0..=255);
                    }
                    for _ in 0..256 {
                        seed = seed
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407);
                        inputs.push(seed & mask);
                    }
                    for input in inputs {
                        let input = input & mask;
                        let expected = if signed {
                            let n = ((input << (64 - bits)) as i64) >> (64 - bits);
                            if remainder {
                                n % divisor as i64
                            } else {
                                n / divisor as i64
                            }
                        } else if remainder {
                            (input % divisor) as i64
                        } else {
                            (input / divisor) as i64
                        };
                        assert_eq!(
                            eval::evaluate(&machine, &[input as i64]).unwrap(),
                            expected,
                            "{width:?} {op:?}: {input} / {divisor}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn trapping_and_dynamic_divisors_keep_hardware_division() {
    for op in [
        Opcode::DivSigned,
        Opcode::RemSigned,
        Opcode::DivUnsigned,
        Opcode::RemUnsigned,
    ] {
        let signed = matches!(op, Opcode::DivSigned | Opcode::RemSigned);
        for divisor in [None, Some(0)]
            .into_iter()
            .chain(signed.then_some(Some(u64::MAX)))
        {
            assert_eq!(
                divides(&binary(
                    "trapping_division",
                    IntegerWidth::W64,
                    signed,
                    op,
                    0,
                    divisor
                )),
                1
            );
        }
    }
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
fn constant_shift_counts_stay_immediate_after_rust_width_masking() {
    use kfrc_x86::machine::eval;

    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let bits = width.bits(64);
        let mask = u64::MAX >> (64 - bits);
        for (op, signed) in [
            (Opcode::ShiftLeft, false),
            (Opcode::ShiftRightLogical, false),
            (Opcode::ShiftRightArithmetic, true),
        ] {
            for amount in [0u64, 1, u64::from(bits) + 3, 255] {
                let machine = binary("constant_shift", width, signed, op, 0, Some(amount));
                let count = (amount % u64::from(bits)) as u8;
                assert!(
                    machine
                        .instructions
                        .iter()
                        .any(|instruction| match (op, instruction) {
                            (Opcode::ShiftLeft, MachineInstr::ShlImm { count: actual, .. })
                            | (
                                Opcode::ShiftRightLogical,
                                MachineInstr::LShrImm { count: actual, .. },
                            )
                            | (
                                Opcode::ShiftRightArithmetic,
                                MachineInstr::ShrImm { count: actual, .. },
                            ) => {
                                *actual == count
                            }
                            _ => false,
                        }),
                    "{width:?} {op:?} {amount}: {:?}",
                    machine.instructions
                );
                assert!(!machine.instructions.iter().any(|instruction| matches!(
                    instruction,
                    MachineInstr::Shl { .. } | MachineInstr::LShr { .. } | MachineInstr::Shr { .. }
                )));
                for input in [0i64, 1, 7, -3] {
                    let source = (input as u64) & mask;
                    let raw = match op {
                        Opcode::ShiftLeft => source.wrapping_shl(u32::from(count)) & mask,
                        Opcode::ShiftRightLogical => source >> count,
                        Opcode::ShiftRightArithmetic => {
                            let signed_source = ((source << (64 - bits)) as i64) >> (64 - bits);
                            (signed_source >> count) as u64 & mask
                        }
                        _ => unreachable!(),
                    };
                    let expected = if signed {
                        ((raw << (64 - bits)) as i64) >> (64 - bits)
                    } else {
                        raw as i64
                    };
                    assert_eq!(eval::evaluate(&machine, &[input]).unwrap(), expected);
                }
            }
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn native_ordinary_constant_division_matches_c() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::{fs, process::Command};
    let directory =
        std::env::temp_dir().join(format!("kfrc-ordinary-division-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let mut functions = Vec::new();
    let mut declarations = String::from("#include <stdint.h>\n#include <stdio.h>\n");
    let mut checks =
        String::from("int main(void) { uint64_t state = UINT64_C(0x123456789abcdef0);\n");
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let bits = width.bits(64);
        for signed in [false, true] {
            let max = if signed {
                u64::MAX >> (65 - bits)
            } else {
                u64::MAX >> (64 - bits)
            };
            for divisor in [1, 2, 3, 7, 10, 60, 127, 256, 3600, 86400, max, max - 1] {
                if divisor > max {
                    continue;
                }
                for remainder in [false, true] {
                    let op = match (signed, remainder) {
                        (true, false) => Opcode::DivSigned,
                        (true, true) => Opcode::RemSigned,
                        (false, false) => Opcode::DivUnsigned,
                        (false, true) => Opcode::RemUnsigned,
                    };
                    let name = format!("ordinary_{}", functions.len());
                    let ctype = if signed { "int64_t" } else { "uint64_t" };
                    let narrow = format!("{}int{bits}_t", if signed { "" } else { "u" });
                    declarations.push_str(&format!("extern {ctype} {name}({ctype});\n"));
                    functions.push(FunctionCode {
                        function: binary(&name, width, signed, op, 0, Some(divisor)),
                        binding: SymbolBinding::Global { hidden: false },
                    });
                    let operator = if remainder { '%' } else { '/' };
                    checks.push_str(&format!("for (unsigned i=0;i<4096;++i) {{ state=state*UINT64_C(6364136223846793005)+UINT64_C(1442695040888963407); {ctype} n=({narrow})state; {ctype} expected=n {operator} ({ctype})UINT64_C({divisor}); if ({name}(n)!=expected) {{fprintf(stderr, \"{name} mismatch\\n\");return 1;}} }}\n"));
                }
            }
        }
    }
    checks.push_str("return 0;}\n");
    fs::write(directory.join("test.o"), write_object(&functions).unwrap()).unwrap();
    fs::write(directory.join("test.c"), declarations + &checks).unwrap();
    let built = Command::new("cc")
        .args(["-O2", "-no-pie"])
        .arg(directory.join("test.c"))
        .arg(directory.join("test.o"))
        .arg("-o")
        .arg(directory.join("test"))
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let executed = Command::new(directory.join("test")).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    fs::remove_dir_all(directory).unwrap();
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
