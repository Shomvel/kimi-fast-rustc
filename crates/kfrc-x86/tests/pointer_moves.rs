//! Address retyping keeps its graph types while selecting flag-preserving moves.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::{MachineInstr, lower_function};

fn projection(graph: &mut FunctionGraph, source: NodeId, index: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty)
}

fn graph(offset: u64, pressure: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let byte = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let bytes = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: byte,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let name = if pressure {
        "pointer_pressure"
    } else if offset == 0 {
        "pointer_zero"
    } else {
        "pointer_nonzero"
    };
    let mut graph = FunctionGraph::new(
        name,
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
    let zero = graph.add_constant(0, word);
    let mut address = input;
    for index in 0..12 {
        address = graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[address, zero],
            0,
            ValueType::Rust(if index % 2 == 0 { bytes } else { pointer }),
        );
    }
    let mut values = Vec::new();
    for index in 0..if pressure { 12 } else { 1 } {
        let offset = graph.add_constant(u128::from(offset + index * 8), word);
        let address = graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[address, offset],
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
        control = projection(&mut graph, load, 0, ValueType::Control);
        effect = projection(&mut graph, load, 1, ValueType::Effect);
        values.push(projection(&mut graph, load, 2, ValueType::Rust(word)));
    }
    if pressure {
        let target = graph.add_callee(CallTarget {
            symbol: "clobber_pointer_source".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
        });
        let call = graph.add_node(
            Opcode::Call,
            control,
            &[effect, input],
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
            ]),
        );
        control = projection(&mut graph, call, 0, ValueType::Control);
        effect = projection(&mut graph, call, 1, ValueType::Effect);
    }
    let mut sum = values[0];
    for value in values.into_iter().skip(1) {
        sum = graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[sum, value],
            0,
            ValueType::Rust(word),
        );
    }
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, sum],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

#[test]
fn constant_offsets_select_no_arithmetic_and_preserve_graph_types() {
    for offset in [0, 8] {
        let graph = graph(offset, false);
        let before = graph.dump();
        let machine = lower_function(&graph).unwrap();
        assert_eq!(before, graph.dump());
        // A constant pointer offset is the load displacement, not an add.
        assert!(
            !machine
                .instructions
                .iter()
                .any(|instruction| matches!(instruction, MachineInstr::Add { .. }))
        );
        let displacements: Vec<_> = machine
            .instructions
            .iter()
            .filter_map(|instruction| match instruction {
                MachineInstr::LoadIndirect { offset, .. } => Some(u64::try_from(*offset).unwrap()),
                _ => None,
            })
            .collect();
        assert!(!displacements.is_empty());
        assert!(displacements.iter().all(|&d| d == offset), "{displacements:?}");
        assert!(!machine.instructions.iter().any(
            |instruction| matches!(instruction, MachineInstr::Mov { dst, src } if dst == src)
        ));
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn encoded_pointer_moves_preserve_snapshots_across_call_clobbers_and_spills() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::process::Command;
    let functions = [(0, false), (8, false), (0, true)].map(|(offset, pressure)| FunctionCode {
        function: lower_function(&graph(offset, pressure)).unwrap(),
        binding: SymbolBinding::Global { hidden: false },
    });
    let directory = std::env::temp_dir().join(format!("kfrc-pointer-moves-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("main.c");
    let object = directory.join("pointer.o");
    let executable = directory.join("check");
    std::fs::write(&object, write_object(&functions).unwrap()).unwrap();
    std::fs::write(&source, r#"
#include <stdint.h>
extern uint64_t pointer_zero(const uint64_t *);
extern uint64_t pointer_nonzero(const uint64_t *);
extern uint64_t pointer_pressure(const uint64_t *);
void clobber_pointer_source(uint64_t *values) {
    for (unsigned i=0;i<12;i++) values[i] = 0xdeadbeef;
    __asm__ volatile("" ::: "rax","rcx","rdx","rsi","rdi","r8","r9","r10","r11","memory","cc");
}
int main(void) {
    for (uint64_t seed=0;seed<256;seed++) {
        uint64_t values[12], expected=0;
        for (unsigned i=0;i<12;i++) { values[i] = seed * 0x10101010101ULL + i; expected += values[i]; }
        if (pointer_zero(values) != values[0] || pointer_nonzero(values) != values[1]) return 1;
        if (pointer_pressure(values) != expected) return 2;
        for (unsigned i=0;i<12;i++) if (values[i] != 0xdeadbeef) return 3;
    }
    return 0;
}
"#).unwrap();
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
    let ran = Command::new(&executable).output().unwrap();
    assert!(ran.status.success(), "machine mismatch {:?}", ran.status);
    std::fs::remove_dir_all(directory).unwrap();
}
