//! Compare-only branch consumers need flags, not a materialized Boolean.
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TypeShape};
use kfrc_son::{
    CallTarget, ComparePredicate as P, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType,
    lower_layout, verify_graph,
};
use kfrc_x86::{MachineFunction, MachineInstr, lower_function};

const PREDICATES: [P; 10] = [
    P::Equal,
    P::NotEqual,
    P::SignedLess,
    P::SignedLessEqual,
    P::SignedGreater,
    P::SignedGreaterEqual,
    P::UnsignedLess,
    P::UnsignedLessEqual,
    P::UnsignedGreater,
    P::UnsignedGreaterEqual,
];

#[derive(Default, Clone, Copy)]
struct Options {
    nots: usize,
    zero_rhs: bool,
    shared: bool,
    nested: bool,
    pressure: bool,
    call_before_branch: bool,
    /// Compare `CONSTANT_LHS` against the first argument instead.
    constant_lhs: bool,
}

const CONSTANT_LHS: i64 = 127;

fn projection(g: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    g.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

fn fixture(name: &str, predicate: P, options: Options) -> MachineFunction {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let count = if options.pressure { 20 } else { 2 };
    let mut g = FunctionGraph::new(
        name,
        FunctionAbi {
            args: vec![PassMode::Direct; count],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    g.level = GraphLevel::Specialized;
    let mut outputs = vec![ValueType::Control, ValueType::Effect];
    outputs.extend((0..count).map(|_| ValueType::Rust(word)));
    let start = g.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(outputs),
    );
    g.start = start;
    let mut control = projection(&mut g, start, 0, ValueType::Control);
    let mut effect = projection(&mut g, start, 1, ValueType::Effect);
    let args: Vec<_> = (0..count)
        .map(|i| projection(&mut g, start, i as u32 + 2, ValueType::Rust(word)))
        .collect();
    let zero = g.add_constant(0, word);
    let rhs = if options.zero_rhs { zero } else { args[1] };
    let operands = if options.constant_lhs {
        [g.add_constant(CONSTANT_LHS as u128, word), args[0]]
    } else {
        [args[0], rhs]
    };
    let compared = g.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &operands,
        predicate as u32,
        ValueType::Rust(boolean),
    );
    let mut condition = compared;
    for _ in 0..options.nots {
        condition = g.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[condition],
            0,
            ValueType::Rust(boolean),
        );
    }
    if options.call_before_branch {
        let target = g.add_callee(CallTarget {
            symbol: "branch_touch".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct; 2],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        let call = g.add_node(
            Opcode::Call,
            control,
            &[effect, args[0], args[1]],
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
            ]),
        );
        control = projection(&mut g, call, 0, ValueType::Control);
        effect = projection(&mut g, call, 1, ValueType::Effect);
    }
    if options.nested {
        let outer = g.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[args[0], zero],
            P::NotEqual as u32,
            ValueType::Rust(boolean),
        );
        let branch = g.add_node(
            Opcode::If,
            control,
            &[outer],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        control = projection(&mut g, branch, 0, ValueType::Control);
        let other = projection(&mut g, branch, 1, ValueType::Control);
        g.add_node(
            Opcode::Return,
            other,
            &[effect, zero],
            0,
            ValueType::Control,
        );
    }
    let branch = g.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let mut sum = zero;
    if options.pressure {
        for &arg in &args {
            sum = g.add_node(
                Opcode::Add,
                NodeId::NONE,
                &[sum, arg],
                0,
                ValueType::Rust(word),
            );
        }
    }
    let shared = if options.shared {
        Some(g.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[compared],
            0,
            ValueType::Rust(word),
        ))
    } else {
        None
    };
    for (slot, value) in [17, 29].into_iter().enumerate() {
        let exit = projection(&mut g, branch, slot as u32, ValueType::Control);
        let constant = g.add_constant(value, word);
        let mut result = constant;
        if options.pressure {
            result = g.add_node(
                Opcode::Add,
                NodeId::NONE,
                &[result, sum],
                0,
                ValueType::Rust(word),
            );
        }
        if let Some(shared) = shared {
            result = g.add_node(
                Opcode::Add,
                NodeId::NONE,
                &[result, shared],
                0,
                ValueType::Rust(word),
            );
        }
        g.add_node(
            Opcode::Return,
            exit,
            &[effect, result],
            0,
            ValueType::Control,
        );
    }
    verify_graph(&mut g).unwrap();
    lower_layout(&mut g).unwrap();
    verify_graph(&mut g).unwrap();
    lower_function(&g).unwrap()
}

fn predicate(p: P, a: i64, b: i64) -> bool {
    match p {
        P::Equal => a == b,
        P::NotEqual => a != b,
        P::SignedLess => a < b,
        P::SignedLessEqual => a <= b,
        P::SignedGreater => a > b,
        P::SignedGreaterEqual => a >= b,
        P::UnsignedLess => (a as u64) < b as u64,
        P::UnsignedLessEqual => (a as u64) <= b as u64,
        P::UnsignedGreater => (a as u64) > b as u64,
        P::UnsignedGreaterEqual => (a as u64) >= b as u64,
    }
}

fn materialized(machine: &MachineFunction) -> usize {
    machine
        .instructions
        .iter()
        .filter(|i| matches!(i, MachineInstr::SetCc { .. }))
        .count()
}

fn check(machine: &MachineFunction, p: P, options: Options) {
    let values = [i64::MIN, i64::MIN + 1, -1, 0, 1, 127, 128, i64::MAX];
    for a in values {
        for b in values {
            let mut args = vec![a, b];
            if options.pressure {
                args.extend((2..20).map(|i| (i * 7919) as i64));
            }
            let truth = if options.constant_lhs {
                predicate(p, CONSTANT_LHS, a)
            } else {
                predicate(p, a, if options.zero_rhs { 0 } else { b })
            };
            let take = truth ^ (options.nots % 2 != 0);
            let mut expected: i64 = if take { 17 } else { 29 };
            if options.shared {
                expected += i64::from(truth);
            }
            if options.pressure {
                expected = args.iter().fold(expected, |sum, &x| sum.wrapping_add(x));
            }
            if options.nested && a == 0 {
                expected = 0;
            }
            assert_eq!(
                kfrc_x86::machine::eval::evaluate(machine, &args).unwrap(),
                expected,
                "{p:?} a={a} b={b} nots={}",
                options.nots
            );
        }
    }
}

#[test]
fn direct_integer_conditions_and_not_polarity_preserve_boundary_values() {
    for p in PREDICATES {
        for nots in 0..=2 {
            for zero_rhs in [false, true] {
                let options = Options {
                    nots,
                    zero_rhs,
                    ..Options::default()
                };
                let machine = fixture("direct", p, options);
                assert_eq!(materialized(&machine), 0, "{p:?}");
                machine.emit_machine_code().unwrap();
                check(&machine, p, options);
            }
        }
    }
}

#[test]
fn shared_boolean_stays_materialized() {
    let options = Options {
        shared: true,
        nots: 1,
        ..Options::default()
    };
    let machine = fixture("retained", P::UnsignedLess, options);
    assert!(materialized(&machine) >= 1);
    check(&machine, P::UnsignedLess, options);
}

#[test]
fn comparison_used_only_in_a_nested_block_sinks_into_its_branch() {
    let options = Options {
        nested: true,
        ..Options::default()
    };
    let machine = fixture("sunk", P::UnsignedLess, options);
    assert_eq!(materialized(&machine), 0);
    check(&machine, P::UnsignedLess, options);
}

#[test]
fn direct_branch_preserves_live_values_under_register_pressure() {
    let options = Options {
        pressure: true,
        nots: 1,
        ..Options::default()
    };
    let machine = fixture("pressure", P::SignedGreaterEqual, options);
    assert_eq!(materialized(&machine), 0);
    check(&machine, P::SignedGreaterEqual, options);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn encoded_direct_conditions_match_c_comparisons() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::{fs, process::Command};
    let mut functions = Vec::new();
    let mut declarations = String::new();
    let mut checks = String::new();
    for (index, p) in PREDICATES.into_iter().enumerate() {
        for (case, (nots, zero_rhs, call_before_branch)) in [
            (0, false, false),
            (1, false, false),
            (0, true, false),
            (1, false, true),
        ]
        .into_iter()
        .enumerate()
        {
            let name = format!("branch_{index}_{case}");
            let machine = fixture(
                &name,
                p,
                Options {
                    nots,
                    zero_rhs,
                    call_before_branch,
                    ..Options::default()
                },
            );
            assert_eq!(materialized(&machine), 0);
            if call_before_branch {
                assert_eq!(
                    machine
                        .instructions
                        .iter()
                        .filter(|i| matches!(i, MachineInstr::Call { .. }))
                        .count(),
                    1
                );
            }
            functions.push(FunctionCode {
                function: machine,
                binding: SymbolBinding::Global { hidden: false },
            });
            declarations.push_str(&format!("extern uint64_t {name}(uint64_t,uint64_t);\n"));
            let (cast, op) = match p {
                P::Equal => ("uint64_t", "=="),
                P::NotEqual => ("uint64_t", "!="),
                P::SignedLess => ("int64_t", "<"),
                P::SignedLessEqual => ("int64_t", "<="),
                P::SignedGreater => ("int64_t", ">"),
                P::SignedGreaterEqual => ("int64_t", ">="),
                P::UnsignedLess => ("uint64_t", "<"),
                P::UnsignedLessEqual => ("uint64_t", "<="),
                P::UnsignedGreater => ("uint64_t", ">"),
                P::UnsignedGreaterEqual => ("uint64_t", ">="),
            };
            let rhs = if zero_rhs { "0" } else { "b" };
            checks.push_str(&format!("if ({name}(a,b) != (((({cast})a {op} ({cast}){rhs}) ^ {nots}) ? 17u : 29u)) return 1;\n"));
        }
    }
    let dir = std::env::temp_dir().join(format!("kfrc-direct-branches-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let object = dir.join("branches.o");
    fs::write(&object, write_object(&functions).unwrap()).unwrap();
    let source = dir.join("main.c");
    fs::write(&source, format!("#include <stdint.h>\n{declarations}\n__attribute__((noinline)) uint64_t branch_touch(uint64_t a,uint64_t b) {{ __asm__ volatile(\"cmp $1, %%rax\" ::: \"cc\"); return a+b; }}\nint main(void) {{ uint64_t values[]={{0,1,127,128,UINT64_MAX,INT64_MAX,UINT64_C(1)<<63}}; for(unsigned i=0;i<7;i++) for(unsigned j=0;j<7;j++) {{ uint64_t a=values[i],b=values[j]; {checks} }} return 0; }}")).unwrap();
    let executable = dir.join("check");
    let linked = Command::new("cc")
        .arg("-O2")
        .arg(&source)
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    assert!(Command::new(&executable).status().unwrap().success());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn constant_left_operands_swap_into_the_immediate_with_a_mirrored_condition() {
    for p in PREDICATES {
        for shared in [false, true] {
            let options = Options {
                constant_lhs: true,
                shared,
                ..Options::default()
            };
            let machine = fixture("constant_lhs", p, options);
            // The constant is the compare's immediate, never a register load.
            assert!(
                !machine
                    .instructions
                    .iter()
                    .any(|i| matches!(i, MachineInstr::MovImm { value, .. } if *value == CONSTANT_LHS)),
                "{p:?} shared={shared}"
            );
            check(&machine, p, options);
        }
    }
}
