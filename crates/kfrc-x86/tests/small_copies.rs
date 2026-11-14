//! Exact-range small copies and authenticated byte-equality loads.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::{lower_function, MachineInstr};

fn projection(graph: &mut FunctionGraph, source: NodeId, index: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty)
}

/// `memmove(dst, src, size)` returning the call's pointer result.
fn graph(symbol: &str, size: u64) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut graph = FunctionGraph::new(
        "small_copies",
        FunctionAbi {
            args: vec![PassMode::Direct, PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Layout;
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(pointer),
            ValueType::Rust(pointer),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let destination = projection(&mut graph, start, 2, ValueType::Rust(pointer));
    let source = projection(&mut graph, start, 3, ValueType::Rust(pointer));
    let count = graph.add_constant(u128::from(size), word);
    let callee = graph.add_callee(CallTarget {
        symbol: symbol.into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct; 3],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    let call = graph.add_node(
        Opcode::Call,
        control,
        &[effect, destination, source, count],
        callee,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(pointer),
        ]),
    );
    let control = projection(&mut graph, call, 0, ValueType::Control);
    let effect = projection(&mut graph, call, 1, ValueType::Effect);
    let result = projection(&mut graph, call, 2, ValueType::Rust(pointer));
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, result],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

#[test]
fn small_copies_load_every_chunk_before_storing() {
    for symbol in ["memmove", "memcpy"] {
        for (size, chunks) in [(8u64, vec![8u8]), (15, vec![8, 4, 2, 1]), (32, vec![8; 4])] {
            let machine = lower_function(&graph(symbol, size)).unwrap();
            assert!(
                machine.symbols.iter().all(|name| name != symbol),
                "{symbol} {size}"
            );
            let accesses: Vec<_> = machine
                .instructions
                .iter()
                .filter_map(|instruction| match instruction {
                    MachineInstr::LoadIndirect { bytes, offset, .. } => {
                        Some((true, *bytes, *offset))
                    }
                    MachineInstr::StoreIndirect { bytes, offset, .. } => {
                        Some((false, *bytes, *offset))
                    }
                    _ => None,
                })
                .collect();
            let loads = accesses.iter().take_while(|(load, ..)| *load).count();
            assert_eq!(loads, chunks.len(), "{symbol} {size}: {accesses:?}");
            assert_eq!(
                accesses.len(),
                2 * chunks.len(),
                "{symbol} {size}: {accesses:?}"
            );
            let widths: Vec<_> = accesses[..loads]
                .iter()
                .map(|(_, bytes, _)| *bytes)
                .collect();
            assert_eq!(widths, chunks);
        }
    }
}

#[test]
fn larger_copies_stay_library_calls() {
    let machine = lower_function(&graph("memmove", 33)).unwrap();
    assert!(machine.symbols.iter().any(|name| name == "memmove"));
}

fn equality_graph(size: u64, trusted: bool) -> FunctionGraph {
    let mut graph = graph("bcmp", size);
    let integer = graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W32,
            signed: true,
        },
        Some(SizeAlign { size: 4, align: 4 }),
    );
    let call = graph
        .ids()
        .find(|&node| graph.op(node) == Opcode::Call)
        .unwrap();
    graph.set_value_type(
        call,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(integer),
        ]),
    );
    let result = graph.find_projection(call, 2).unwrap();
    graph.set_value_type(result, ValueType::Rust(integer));
    if trusted {
        let target = graph.add_runtime_comparison(RuntimeComparison::Bcmp);
        graph.set_payload(call, target);
    }
    verify_graph(&mut graph).unwrap();
    graph
}

#[test]
fn small_byte_equality_is_loads_xors_and_one_compare() {
    for (size, chunks) in [(1u64, 1usize), (4, 1), (12, 2), (32, 4)] {
        let machine = lower_function(&equality_graph(size, true)).unwrap();
        assert!(machine.symbols.iter().all(|name| name != "bcmp"), "{size}");
        let count =
            |f: fn(&MachineInstr) -> bool| machine.instructions.iter().filter(|i| f(i)).count();
        assert_eq!(
            count(|i| matches!(i, MachineInstr::LoadIndirect { .. })),
            2 * chunks
        );
        assert_eq!(
            count(
                |i| matches!(i, MachineInstr::Xor { dst, src: kfrc_x86::machine::x86::Operand::Reg(src) } if dst != src)
            ),
            chunks
        );
        assert_eq!(count(|i| matches!(i, MachineInstr::SetCc { .. })), 1);
    }
    let machine = lower_function(&equality_graph(33, true)).unwrap();
    assert!(machine.symbols.iter().any(|name| name == "bcmp"));
}

#[test]
fn same_named_untrusted_equality_remains_a_call_without_memory_reads() {
    for size in [0, 1, 7, 32] {
        let machine = lower_function(&equality_graph(size, false)).unwrap();
        assert!(machine.symbols.iter().any(|name| name == "bcmp"), "{size}");
        assert!(
            machine.instructions.iter().all(|instruction| !matches!(
                instruction,
                MachineInstr::LoadIndirect { .. } | MachineInstr::StoreIndirect { .. }
            )),
            "an unauthenticated call must not read its pointer operands: {size}",
        );
    }
}

#[test]
fn edited_runtime_comparison_target_loses_expansion_authority() {
    for change_abi in [false, true] {
        let mut graph = equality_graph(7, true);
        let call = graph
            .ids()
            .find(|&node| graph.op(node) == Opcode::Call)
            .unwrap();
        let target = graph.payload(call) as usize;
        if change_abi {
            graph.callees[target].abi.c_variadic = true;
        } else {
            graph.callees[target].symbol = "user_byte_comparison".into();
        }
        assert_eq!(graph.runtime_comparison(target as u32), None);
        verify_graph(&mut graph).unwrap();
        let machine = lower_function(&graph).unwrap();
        assert!(machine.symbols.contains(&graph.callees[target].symbol));
        assert!(machine
            .instructions
            .iter()
            .all(|instruction| !matches!(instruction, MachineInstr::LoadIndirect { .. })));
    }
}

#[test]
fn every_small_equality_load_is_paired_and_inside_the_compared_range() {
    let overlap = std::env::var("KFRC_SMALL_EQUALITY_OVERLAP").as_deref() == Ok("1");
    for size in 0..=32u64 {
        let graph = equality_graph(size, true);
        let before = graph.dump();
        let machine = lower_function(&graph).unwrap();
        assert_eq!(graph.dump(), before);
        assert!(machine.symbols.iter().all(|name| name != "bcmp"));
        let loads: Vec<_> = machine
            .instructions
            .iter()
            .filter_map(|instruction| match instruction {
                MachineInstr::LoadIndirect { bytes, offset, .. } => Some((*offset, *bytes)),
                _ => None,
            })
            .collect();
        assert_eq!(loads.len() % 2, 0, "size={size}: {loads:?}");
        let mut covered = [false; 32];
        for pair in loads.chunks_exact(2) {
            assert_eq!(pair[0], pair[1], "both operands use the same chunk");
            let (offset, bytes) = pair[0];
            assert!(offset >= 0);
            assert!(matches!(bytes, 1 | 2 | 4 | 8));
            let end = offset as usize + usize::from(bytes);
            assert!(end <= size as usize, "size={size}: {loads:?}");
            covered[offset as usize..end].fill(true);
        }
        assert!(covered[..size as usize].iter().all(|covered| *covered));
        assert!(covered[size as usize..].iter().all(|covered| !covered));
        // These are exactly the sizes whose full-width tail eliminates at
        // least one chunk. Ties must continue using the original plan.
        let reduced = overlap && matches!(size, 7 | 11 | 13..=15 | 19 | 21..=23 | 27 | 29..=31);
        let original_chunks = size / 8 + u64::from((size % 8).count_ones());
        assert_eq!(
            (loads.len() as u64 / 2) < original_chunks,
            reduced,
            "size={size}: {loads:?}"
        );
        if !reduced {
            let mut expected_offset = 0;
            for pair in loads.chunks_exact(2) {
                assert_eq!(pair[0].0 as u64, expected_offset);
                expected_offset += u64::from(pair[0].1);
            }
            assert_eq!(expected_offset, size);
        }
        if overlap && size == 7 {
            assert_eq!(loads, [(0, 4), (0, 4), (3, 4), (3, 4)]);
        }
        if overlap && size == 15 {
            assert_eq!(loads, [(0, 8), (0, 8), (7, 8), (7, 8)]);
        }
    }
}

#[test]
fn dynamic_authenticated_equality_retains_its_call_and_reads_no_bytes_inline() {
    let mut graph = equality_graph(7, true);
    let call = graph
        .ids()
        .find(|&node| graph.op(node) == Opcode::Call)
        .unwrap();
    let count = graph.input(call, 3);
    let word = graph.rust_type(count).unwrap();
    let ValueType::Tuple(mut parameters) = graph.value_type(graph.start).clone() else {
        unreachable!();
    };
    parameters.push(ValueType::Rust(word));
    let start = graph.start;
    graph.set_value_type(start, ValueType::Tuple(parameters));
    graph.abi.args.push(PassMode::Direct);
    let dynamic_count = projection(&mut graph, start, 4, ValueType::Rust(word));
    graph.set_input(call, 3, dynamic_count);
    verify_graph(&mut graph).unwrap();
    let machine = lower_function(&graph).unwrap();
    assert!(machine.symbols.iter().any(|name| name == "bcmp"));
    assert!(machine.instructions.iter().all(|instruction| !matches!(
        instruction,
        MachineInstr::LoadIndirect { .. } | MachineInstr::StoreIndirect { .. }
    )));
}

fn marker(
    graph: &mut FunctionGraph,
    symbol: &str,
    control: NodeId,
    effect: NodeId,
) -> (NodeId, NodeId) {
    let callee = graph.add_callee(CallTarget {
        symbol: symbol.into(),
        abi: FunctionAbi {
            args: vec![],
            ret: PassMode::Ignore,
            c_variadic: false,
        },
    });
    let call = graph.add_node(
        Opcode::Call,
        control,
        &[effect],
        callee,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    (
        projection(graph, call, 0, ValueType::Control),
        projection(graph, call, 1, ValueType::Effect),
    )
}

#[test]
fn equality_loads_remain_between_the_original_effectful_observers() {
    for size in [0, 7, 15, 32] {
        let mut graph = equality_graph(size, true);
        let comparison = graph
            .ids()
            .find(|&node| graph.op(node) == Opcode::Call)
            .unwrap();
        let control = graph.control(comparison);
        let effect = graph.input(comparison, 0);
        let (control, effect) = marker(&mut graph, "before_equality", control, effect);
        graph.set_control(comparison, control);
        graph.set_input(comparison, 0, effect);
        let returned = graph
            .ids()
            .find(|&node| graph.op(node) == Opcode::Return)
            .unwrap();
        let control = graph.control(returned);
        let effect = graph.input(returned, 0);
        let (control, effect) = marker(&mut graph, "after_equality", control, effect);
        graph.set_control(returned, control);
        graph.set_input(returned, 0, effect);
        verify_graph(&mut graph).unwrap();
        let before = graph.dump();
        let machine = lower_function(&graph).unwrap();
        assert_eq!(graph.dump(), before);
        let call_position = |symbol: &str| {
            machine.instructions.iter().position(|instruction| {
                matches!(instruction, MachineInstr::Call { target } if machine.symbols[*target as usize] == symbol)
            }).unwrap()
        };
        let first = call_position("before_equality");
        let last = call_position("after_equality");
        assert!(first < last);
        for (position, instruction) in machine.instructions.iter().enumerate() {
            if matches!(instruction, MachineInstr::LoadIndirect { .. }) {
                assert!(first < position && position < last, "size={size}");
            }
        }
    }
}
