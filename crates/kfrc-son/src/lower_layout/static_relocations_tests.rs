use super::*;
use crate::{Interpreter, Outcome, verify_graph};
use kfrc_body::{FunctionAbi, PassMode, RecordedTypes, ScalarValidRange, ScalarValidity};

struct Fixture {
    graph: FunctionGraph,
    control: NodeId,
    effect: NodeId,
    byte: TyId,
    word: TyId,
    pointer: TyId,
}

impl Fixture {
    fn new() -> Self {
        let mut types = RecordedTypes::new();
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "relocation_read",
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
        let control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[graph.start],
            0,
            ValueType::Control,
        );
        let effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[graph.start],
            1,
            ValueType::Effect,
        );
        graph.data_objects.push(DataObject {
            symbol: "target".into(),
            bytes: vec![7, 11, b'=', 91, 0xff, 6, 3, 19],
            align: 8,
            ..Default::default()
        });
        graph.data_objects.push(DataObject {
            symbol: "holder".into(),
            bytes: vec![0; 24],
            align: 8,
            relocations: vec![DataRelocation {
                offset: 8,
                target: "target".into(),
                addend: 2,
            }],
            ..Default::default()
        });
        Self {
            graph,
            control,
            effect,
            byte,
            word,
            pointer,
        }
    }

    fn symbol(&mut self, name: &str, addend: i64) -> NodeId {
        let payload = self.graph.symbols.len() as u32;
        self.graph.symbols.push((name.into(), addend));
        self.graph.add_node(
            Opcode::SymbolAddress,
            NodeId::NONE,
            &[],
            payload,
            ValueType::Rust(self.pointer),
        )
    }

    fn load(&mut self, address: NodeId, ty: TyId) -> NodeId {
        let load = self.graph.add_node(
            Opcode::Load,
            self.control,
            &[self.effect, address],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[load],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[load],
            1,
            ValueType::Effect,
        );
        self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[load],
            2,
            ValueType::Rust(ty),
        )
    }

    fn finish(mut self, result: NodeId, shared: bool) -> FunctionGraph {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, result],
            0,
            ValueType::Control,
        );
        if shared {
            self.graph.program_data = std::sync::Arc::new(
                std::mem::take(&mut self.graph.data_objects)
                    .into_iter()
                    .map(|object| (object.symbol.clone(), object))
                    .collect(),
            );
        }
        verify_graph(&mut self.graph).unwrap();
        self.graph
    }
}

fn execute(graph: &FunctionGraph) -> Outcome {
    Interpreter::new(graph)
        .run(&[], &mut |_: &str, _: &[u64]| unreachable!())
        .unwrap()
}

#[test]
fn complete_relocation_reads_preserve_addresses_and_addends_in_both_data_owners() {
    for shared in [false, true] {
        for integer in [false, true] {
            for addend in [-7, 0, 2, i64::MAX] {
                let mut f = Fixture::new();
                f.graph.data_objects[1].relocations[0].addend = addend;
                let address = f.symbol("holder", 8);
                let result = f.load(address, if integer { f.word } else { f.pointer });
                let mut graph = f.finish(result, shared);
                let before = graph.clone();
                assert_eq!(run(&mut graph), 1);
                graph.compact();
                verify_graph(&mut graph).unwrap();
                assert_eq!(execute(&graph), execute(&before));
                assert!(!graph.ids().any(|n| graph.op(n) == Opcode::Load));
                assert_eq!(graph.data_objects, before.data_objects);
                assert_eq!(graph.program_data, before.program_data);
            }
        }
    }
}

#[test]
fn relocation_backed_byte_reads_fold_in_one_bounded_query() {
    for shared in [false, true] {
        for integer in [false, true] {
            for depth in [1, 3, 40] {
                let mut f = Fixture::new();
                let mut root = "holder".to_owned();
                for level in 1..depth {
                    let symbol = format!("holder-{level}");
                    f.graph.data_objects.push(DataObject {
                        symbol: symbol.clone(),
                        bytes: vec![0; 24],
                        align: 8,
                        relocations: vec![DataRelocation {
                            offset: 8,
                            target: root,
                            addend: 8,
                        }],
                        ..Default::default()
                    });
                    root = symbol;
                }
                let mut pointer = f.symbol(&root, 8);
                for _ in 0..depth {
                    pointer = f.load(pointer, if integer { f.word } else { f.pointer });
                    if integer {
                        pointer = f.graph.add_node(
                            Opcode::IntCast,
                            NodeId::NONE,
                            &[pointer],
                            0,
                            ValueType::Rust(f.pointer),
                        );
                    }
                }
                let result = f.load(pointer, f.byte);
                let mut graph = f.finish(result, shared);
                let before = graph.clone();
                // Integer casts consume the same query budget as projections.
                let expected = if depth == 40 {
                    if integer { 16 } else { 32 }
                } else {
                    depth + 1
                };
                assert_eq!(run(&mut graph), expected);
                graph.compact();
                verify_graph(&mut graph).unwrap();
                assert_eq!(execute(&graph), execute(&before));
                assert_eq!(execute(&graph), Outcome::Return(u64::from(b'=')));
                assert_eq!(
                    graph.ids().filter(|&n| graph.op(n) == Opcode::Load).count(),
                    depth + 1 - expected
                );
            }
        }
    }
}

#[test]
fn partial_relocations_unknown_targets_and_unstable_objects_are_not_pointer_constants() {
    for reason in 0..10 {
        let mut f = Fixture::new();
        let mut ty = f.word;
        let mut offset = 8;
        match reason {
            0 => ty = f.byte,
            1 => offset = 9,
            2 => f.graph.data_objects[1].writable = true,
            3 => f.graph.data_objects[1].binding = DataBinding::Global { hidden: false },
            4 => f.graph.data_objects[0].binding = DataBinding::Global { hidden: false },
            5 => f.graph.data_objects[1].relocations[0].target = "unknown".into(),
            6 => f.graph.data_objects[1].relocations.push(DataRelocation {
                offset: 9,
                target: "target".into(),
                addend: 0,
            }),
            7 => f.graph.data_objects[1].relocations[0].offset = u64::MAX,
            8 => f.graph.data_objects[1].bytes.truncate(12),
            9 => f.graph.oracle.record_scalar_validity(
                f.word,
                ScalarValidity::Scalar(ScalarValidRange::new(64, 1, u64::MAX as u128).unwrap()),
            ),
            _ => unreachable!(),
        }
        let address = f.symbol("holder", offset);
        let result = f.load(address, ty);
        let mut graph = f.finish(result, false);
        // Cases6/7/8 are intentionally malformed relocation layouts. Inspect the
        // conservative matcher without interpreting malformed storage bytes.
        let before = graph.dump();
        assert_eq!(run(&mut graph), 0, "reason {reason}");
        assert_eq!(graph.dump(), before);
    }
    for tls in ["holder", "target"] {
        let mut f = Fixture::new();
        let address = f.symbol("holder", 8);
        let result = f.load(address, f.word);
        let mut graph = f.finish(result, false);
        graph
            .data_objects
            .iter_mut()
            .find(|object| object.symbol == tls)
            .unwrap()
            .thread_local = true;
        let before = graph.dump();
        assert_eq!(run(&mut graph), 0);
        assert_eq!(graph.dump(), before);
    }
}

#[test]
fn mutable_target_address_folds_but_its_observed_byte_stays_a_load() {
    let mut f = Fixture::new();
    f.graph.data_objects[0].writable = true;
    let address = f.symbol("holder", 8);
    let pointer = f.load(address, f.pointer);
    let result = f.load(pointer, f.byte);
    let mut graph = f.finish(result, false);
    let before = graph.clone();
    assert_eq!(run(&mut graph), 1);
    graph.compact();
    verify_graph(&mut graph).unwrap();
    assert_eq!(execute(&graph), execute(&before));
    assert_eq!(
        graph.ids().filter(|&n| graph.op(n) == Opcode::Load).count(),
        1
    );
}

#[test]
fn cyclic_relocation_address_queries_exhaust_one_shared_bound() {
    let mut f = Fixture::new();
    let address = f.symbol("holder", 8);
    let pointer = f.load(address, f.pointer);
    let result = f.load(pointer, f.pointer);
    let mut graph = f.finish(result, false);
    let load = graph.ids().find(|&n| graph.op(n) == Opcode::Load).unwrap();
    graph.set_input(load, 1, pointer);
    let mut remaining = 32;
    assert!(super::address(&graph, result, &mut remaining).is_none());
    assert_eq!(remaining, 0);
}
