//! Physical enum branching preserves refined pointer control dependencies.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::lower_function;

fn projection(g: &mut FunctionGraph, source: NodeId, index: u32, ty: ValueType) -> NodeId {
    g.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty)
}

fn enum_graph(name: &str, pointer_option: bool, niche: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let pointer = types.intern(
        TypeShape::Reference {
            mutability: Mutability::Not,
            pointee: word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let enumeration = types.intern(
        TypeShape::Adt { name: name.into() },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let count = if pointer_option { 2 } else { 3 };
    let mut variants = vec![vec![]; count];
    if niche {
        variants[count - 1] = vec![FieldLayout {
            ty: pointer,
            offset: 0,
        }];
    }
    types.record_aggregate(
        enumeration,
        AggregateLayout {
            fields: vec![],
            variants,
        },
    );
    types.record_semantic_enum(
        enumeration,
        if pointer_option {
            vec![0, 1]
        } else {
            vec![3, 9, 27]
        },
    );
    types.enums.insert(
        enumeration,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: if pointer_option { pointer } else { word },
                offset: 0,
            }),
            encoding: if niche {
                TagEncoding::Niche {
                    untagged_variant: (count - 1) as u32,
                    niche_variants: 0..=(count - 2) as u32,
                    niche_start: if pointer_option { 0 } else { u64::MAX as u128 },
                }
            } else {
                TagEncoding::Direct {
                    discriminants: vec![3, 9, 27],
                }
            },
        },
    );
    let mut graph = FunctionGraph::new(
        name,
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Specialized;
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(enumeration),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let input = projection(&mut graph, start, 2, ValueType::Rust(enumeration));
    graph
        .variant_switches
        .push(if pointer_option { vec![1] } else { vec![2, 0] });
    let branch = graph.add_node(
        Opcode::VariantSwitch,
        control,
        &[input],
        0,
        ValueType::Tuple(vec![ValueType::Control; if pointer_option { 2 } else { 3 }]),
    );
    if pointer_option {
        let some = projection(&mut graph, branch, 0, ValueType::Control);
        let none = projection(&mut graph, branch, 1, ValueType::Control);
        let witness = graph.add_node(
            Opcode::VariantWitness,
            some,
            &[input, some],
            1,
            ValueType::Witness,
        );
        graph.variant_fields.push((1, 0));
        let pointer = graph.add_node(
            Opcode::VariantField,
            NodeId::NONE,
            &[input, witness],
            0,
            ValueType::Rust(pointer),
        );
        let load = graph.add_node(
            Opcode::LoadValue,
            some,
            &[effect, pointer],
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
        let zero = graph.add_constant(0, word);
        let entry_effect = graph.find_projection(start, 1).unwrap();
        graph.add_node(
            Opcode::Return,
            none,
            &[entry_effect, zero],
            0,
            ValueType::Control,
        );
    } else {
        for (index, result) in [27, 3, 9].into_iter().enumerate() {
            let control = projection(&mut graph, branch, index as u32, ValueType::Control);
            let result = graph.add_constant(result, word);
            graph.add_node(
                Opcode::Return,
                control,
                &[effect, result],
                0,
                ValueType::Control,
            );
        }
    }
    graph
}

fn layout(mut graph: FunctionGraph) -> FunctionGraph {
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    lower_layout(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    lower_function(&graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

#[test]
fn option_pointer_uses_one_null_test_and_retains_refined_load_control() {
    let graph = layout(enum_graph("refined_option", true, true));
    assert_eq!(
        graph.ids().filter(|&n| graph.op(n) == Opcode::Load).count(),
        2
    );
    assert_eq!(
        graph.ids().filter(|&n| graph.op(n) == Opcode::If).count(),
        1
    );
    assert!(
        !graph
            .ids()
            .any(|n| matches!(graph.op(n), Opcode::Sub | Opcode::BitAnd | Opcode::BitOr))
    );
    assert!(
        !graph
            .ids()
            .any(|n| matches!(graph.op(n), Opcode::VariantWitness | Opcode::Assume))
    );
    let branch = graph.ids().find(|&n| graph.op(n) == Opcode::If).unwrap();
    let guard = graph.find_projection(branch, 0).unwrap();
    assert!(
        graph
            .ids()
            .any(|n| graph.op(n) == Opcode::Load && graph.control(n) == guard)
    );
}

#[test]
fn direct_and_wrapping_niche_switches_do_not_decode_then_recompare() {
    for niche in [false, true] {
        let graph = layout(enum_graph("tag_branch", false, niche));
        assert_eq!(
            graph.ids().filter(|&n| graph.op(n) == Opcode::Load).count(),
            1
        );
        assert_eq!(
            graph.ids().filter(|&n| graph.op(n) == Opcode::If).count(),
            2
        );
        assert!(
            !graph
                .ids()
                .any(|n| matches!(graph.op(n), Opcode::BitAnd | Opcode::BitOr | Opcode::Negate))
        );
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn encoded_refinements_match_c_without_speculating_null_dereferences() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::process::Command;
    let functions = [
        enum_graph("refined_option", true, true),
        enum_graph("direct_switch", false, false),
        enum_graph("niche_switch", false, true),
    ]
    .into_iter()
    .map(|graph| FunctionCode {
        function: lower_function(&layout(graph)).unwrap(),
        binding: SymbolBinding::Global { hidden: false },
    })
    .collect::<Vec<_>>();
    let directory = std::env::temp_dir().join(format!("kfrc-refinement-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let object = directory.join("refinement.o");
    let source = directory.join("main.c");
    let executable = directory.join("check");
    std::fs::write(&object, write_object(&functions).unwrap()).unwrap();
    std::fs::write(&source, r#"
#include <stdint.h>
extern uint64_t refined_option(const uint64_t *);
extern uint64_t direct_switch(uint64_t);
extern uint64_t niche_switch(uint64_t);
int main(void) {
    for (uint64_t seed=0; seed<100; seed++) {
        uint64_t value = seed * 0x1020304050607ULL;
        if (refined_option(0) != 0 || refined_option(&value) != value) return 1;
        if (direct_switch(3) != 3 || direct_switch(9) != 9 || direct_switch(27) != 27) return 2;
        if (niche_switch(UINT64_MAX) != 3 || niche_switch(0) != 9 || niche_switch(value | 1) != 27) return 3;
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
    assert!(ran.status.success(), "machine mismatch: {:?}", ran.status);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn constructor_witness_is_not_a_machine_value_or_move() {
    let mut baseline = None;
    for with_witness in [false, true] {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "ManyLogicalVariants".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            enumeration,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![word]; 128],
            },
        );
        types.record_semantic_enum(enumeration, (0..128).collect());
        let mut graph = FunctionGraph::new(
            "constructor_witness",
            FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
            ]),
        );
        graph.start = start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let arg = projection(&mut graph, start, 2, ValueType::Rust(word));
        let value = graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[arg],
            127,
            ValueType::Rust(enumeration),
        );
        let mut inputs = vec![value];
        if with_witness {
            inputs.push(graph.add_node(
                Opcode::VariantWitness,
                control,
                &[value, control],
                127,
                ValueType::Witness,
            ));
        }
        let field = graph.intern_variant_field(127, 0);
        let result = graph.add_node(
            Opcode::VariantField,
            NodeId::NONE,
            &inputs,
            field,
            ValueType::Rust(word),
        );
        graph.add_node(
            Opcode::Return,
            control,
            &[effect, result],
            0,
            ValueType::Control,
        );
        let graph = layout(graph);
        assert!(!graph.ids().any(|n| matches!(
            graph.op(n),
            Opcode::VariantWitness | Opcode::Assume | Opcode::Load | Opcode::Store | Opcode::If
        )));
        let machine = lower_function(&graph).unwrap();
        assert_eq!(
            &machine.instructions,
            baseline.get_or_insert_with(|| machine.instructions.clone())
        );
        for arg in [0, 1, 97] {
            assert_eq!(
                kfrc_x86::machine::eval::evaluate(&machine, &[arg]).unwrap(),
                arg
            );
        }
    }
}
