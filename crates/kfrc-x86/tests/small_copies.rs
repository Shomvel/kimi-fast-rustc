//! Small constant memmove/memcpy calls expand to loads then stores.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::{MachineInstr, lower_function};

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
            assert!(machine.symbols.iter().all(|name| name != symbol), "{symbol} {size}");
            let accesses: Vec<_> = machine
                .instructions
                .iter()
                .filter_map(|instruction| match instruction {
                    MachineInstr::LoadIndirect { bytes, offset, .. } => Some((true, *bytes, *offset)),
                    MachineInstr::StoreIndirect { bytes, offset, .. } => Some((false, *bytes, *offset)),
                    _ => None,
                })
                .collect();
            let loads = accesses.iter().take_while(|(load, ..)| *load).count();
            assert_eq!(loads, chunks.len(), "{symbol} {size}: {accesses:?}");
            assert_eq!(accesses.len(), 2 * chunks.len(), "{symbol} {size}: {accesses:?}");
            let widths: Vec<_> = accesses[..loads].iter().map(|(_, bytes, _)| *bytes).collect();
            assert_eq!(widths, chunks);
        }
    }
}

#[test]
fn larger_copies_stay_library_calls() {
    let machine = lower_function(&graph("memmove", 33)).unwrap();
    assert!(machine.symbols.iter().any(|name| name == "memmove"));
}

#[test]
fn small_byte_equality_is_loads_xors_and_one_compare() {
    for (size, chunks) in [(1u64, 1usize), (4, 1), (12, 2), (32, 4)] {
        let machine = lower_function(&graph("bcmp", size)).unwrap();
        assert!(machine.symbols.iter().all(|name| name != "bcmp"), "{size}");
        let count = |f: fn(&MachineInstr) -> bool| machine.instructions.iter().filter(|i| f(i)).count();
        assert_eq!(count(|i| matches!(i, MachineInstr::LoadIndirect { .. })), 2 * chunks);
        assert_eq!(
            count(|i| matches!(i, MachineInstr::Xor { dst, src: kfrc_x86::machine::x86::Operand::Reg(src) } if dst != src)),
            chunks
        );
        assert_eq!(count(|i| matches!(i, MachineInstr::SetCc { .. })), 1);
    }
    let machine = lower_function(&graph("bcmp", 33)).unwrap();
    assert!(machine.symbols.iter().any(|name| name == "bcmp"));
}
