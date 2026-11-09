//! Constant pointer offsets become load/store displacements on the typed graph.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::{MachineInstr, lower_function};

fn projection(graph: &mut FunctionGraph, source: NodeId, index: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty)
}

/// Loads a word from `input + offsets[0] + offsets[1] + ...`. With `escape`,
/// first stores the first offset pointer itself to `[input]`.
fn graph(offsets: &[u64], escape: bool) -> FunctionGraph {
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
        "address_modes",
        FunctionAbi {
            args: vec![PassMode::Direct],
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
        ]),
    );
    graph.start = start;
    let mut control = projection(&mut graph, start, 0, ValueType::Control);
    let mut effect = projection(&mut graph, start, 1, ValueType::Effect);
    let input = projection(&mut graph, start, 2, ValueType::Rust(pointer));
    let mut chain = Vec::new();
    let mut address = input;
    for &offset in offsets {
        let offset = graph.add_constant(u128::from(offset), word);
        address = graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[address, offset],
            0,
            ValueType::Rust(pointer),
        );
        chain.push(address);
    }
    if escape {
        let store = graph.add_node(
            Opcode::Store,
            control,
            &[effect, input, chain[0]],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        control = projection(&mut graph, store, 0, ValueType::Control);
        effect = projection(&mut graph, store, 1, ValueType::Effect);
    }
    let load = graph.add_node(
        Opcode::Load,
        control,
        &[effect, address],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    let control = projection(&mut graph, load, 0, ValueType::Control);
    let effect = projection(&mut graph, load, 1, ValueType::Effect);
    let value = projection(&mut graph, load, 2, ValueType::Rust(word));
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

fn loads(machine: &kfrc_x86::MachineFunction) -> Vec<i32> {
    machine
        .instructions
        .iter()
        .filter_map(|instruction| match instruction {
            MachineInstr::LoadIndirect { offset, .. } => Some(*offset),
            _ => None,
        })
        .collect()
}

fn adds(machine: &kfrc_x86::MachineFunction) -> usize {
    machine
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction,
                MachineInstr::Add { .. } | MachineInstr::OffsetAddress { .. }
            )
        })
        .count()
}

#[test]
fn constant_offset_chains_fold_into_one_displacement() {
    for (offsets, displacement) in [
        (&[8u64, 16][..], 24),
        (&[16, (-8i64) as u64][..], 8),
        (&[0, 0x7fff_fff0, 0xf][..], 0x7fff_ffff),
    ] {
        let machine = lower_function(&graph(offsets, false)).unwrap();
        assert_eq!(loads(&machine), [displacement], "{offsets:?}");
        assert_eq!(adds(&machine), 0, "{offsets:?}");
    }
}

#[test]
fn offsets_beyond_imm32_stay_pointer_arithmetic() {
    let machine = lower_function(&graph(&[0x1_0000_0000], false)).unwrap();
    assert_eq!(loads(&machine), [0]);
    assert_eq!(adds(&machine), 1);
    // The fold stops where the running displacement would leave imm32.
    let machine = lower_function(&graph(&[0x7fff_ffff, 1], false)).unwrap();
    assert_eq!(loads(&machine), [1]);
    assert_eq!(adds(&machine), 1);
}

#[test]
fn an_offset_pointer_stored_as_data_is_still_materialized() {
    let machine = lower_function(&graph(&[8, 16], true)).unwrap();
    // `input + 8` is the stored value; the load still uses `[input + 24]`.
    assert_eq!(adds(&machine), 1);
    assert_eq!(loads(&machine), [24]);
    let stores: Vec<_> = machine
        .instructions
        .iter()
        .filter_map(|instruction| match instruction {
            MachineInstr::StoreIndirect { offset, .. } => Some(*offset),
            _ => None,
        })
        .collect();
    assert_eq!(stores, [0]);
}
