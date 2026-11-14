use super::*;
use crate::{Interpreter, Outcome, verify_graph};
use kfrc_body::{DataBinding, DataObject, DataRelocation, FunctionAbi, PassMode, RecordedTypes};

fn make(width: IntegerWidth, symbol_offset: i64, offset: u64, shared_data: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let bytes = u64::from(width.bits(64) / 8);
    let word = types.intern(
        TypeShape::Int {
            width,
            signed: false,
        },
        Some(SizeAlign {
            size: bytes,
            align: bytes,
        }),
    );
    let index = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let pointer = types.intern(TypeShape::PlaceAddress { pointee: word }, None);
    let mut graph = FunctionGraph::new(
        "static_read",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Layout;
    graph.start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let start = graph.start;
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
    let data = DataObject {
        symbol: "immutable".into(),
        bytes: (0..32).map(|n| (n * 19 + 7) as u8).collect(),
        align: 8,
        ..DataObject::default()
    };
    if shared_data {
        graph.program_data = std::sync::Arc::new([(data.symbol.clone(), data)].into());
    } else {
        graph.data_objects.push(data);
    }
    graph.symbols.push(("immutable".into(), symbol_offset));
    let address = graph.add_node(
        Opcode::SymbolAddress,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    let displacement = graph.add_constant(u128::from(offset), index);
    let address = graph.add_node(
        Opcode::PtrOffset,
        NodeId::NONE,
        &[address, displacement],
        0,
        ValueType::Rust(pointer),
    );
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
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        1,
        ValueType::Effect,
    );
    let result = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        2,
        ValueType::Rust(word),
    );
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, result],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    graph
}

fn execute(graph: &FunctionGraph) -> Outcome {
    Interpreter::new(graph)
        .run(&[], &mut |_: &str, _: &[u64]| unreachable!())
        .unwrap()
}

#[test]
fn scalar_reads_match_bytes_at_each_width_and_offset_in_both_data_owners() {
    for shared in [false, true] {
        for width in [
            IntegerWidth::W8,
            IntegerWidth::W16,
            IntegerWidth::W32,
            IntegerWidth::W64,
        ] {
            let bytes = usize::from(width.bits(64) / 8);
            for offset in 0..=32 - bytes {
                // The negative symbol addend is compensated by PtrOffset.
                let mut graph = make(width, -3, offset as u64 + 3, shared);
                let before = graph.clone();
                assert_eq!(run(&mut graph), 1);
                graph.compact();
                verify_graph(&mut graph).unwrap();
                assert_eq!(execute(&graph), execute(&before));
                assert!(!graph.ids().any(|node| graph.op(node) == Opcode::Load));
                assert_eq!(graph.data_objects, before.data_objects);
                assert_eq!(graph.program_data, before.program_data);
            }
        }
    }
}

#[test]
fn relocations_mutability_unknown_and_outside_bytes_remain_loads() {
    for reason in 0..8 {
        let mut graph = make(
            IntegerWidth::W16,
            0,
            if reason == 3 { 31 } else { 8 },
            false,
        );
        match reason {
            0 => graph.data_objects[0].writable = true,
            1 => graph.data_objects[0].relocations.push(DataRelocation {
                offset: 9,
                target: "other".into(),
                addend: 0,
            }),
            2 => graph.data_objects[0].relocations.push(DataRelocation {
                offset: 1,
                target: "other".into(),
                addend: 0,
            }),
            3 => {}
            4 => graph.data_objects.clear(),
            5 => graph.data_objects[0].binding = DataBinding::Global { hidden: false },
            6 => graph.data_objects[0].thread_local = true,
            7 => graph.symbols[0].1 = -32,
            _ => unreachable!(),
        }
        let before = graph.dump();
        assert_eq!(run(&mut graph), 0);
        assert_eq!(graph.dump(), before);
    }
    let mut graph = make(IntegerWidth::W16, 0, 8, false);
    graph.data_objects[0].relocations.push(DataRelocation {
        offset: 16,
        target: "other".into(),
        addend: 0,
    });
    let before = graph.clone();
    assert_eq!(run(&mut graph), 1);
    graph.compact();
    verify_graph(&mut graph).unwrap();
    assert_eq!(execute(&graph), execute(&before));
}
