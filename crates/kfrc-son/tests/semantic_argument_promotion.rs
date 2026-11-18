#[allow(dead_code)]
#[path = "semantic_inline_preparation/fixture.rs"]
mod fixture;

#[path = "semantic_argument_promotion/protected.rs"]
mod protected;

use fixture::{Function, projection};
use kfrc_body::{
    AggregateLayout, FieldLayout, Mutability, SemanticAggregate, SizeAlign, TypeShape,
};
use kfrc_son::{
    FunctionGraph, InterpretError, Interpreter, NodeId, Opcode, SemanticCallHandler,
    SemanticMemory, SemanticOutcome, Value, ValueType, promote_arguments, verify_graph,
};
use std::collections::{BTreeMap, BTreeSet};

fn source(fields: usize, unused: bool) -> Function {
    let mut f = Function::new("private_reader", 2);
    let pointee = if fields == 1 {
        f.word
    } else {
        let ty = f.graph.oracle.intern(
            TypeShape::Tuple(vec![f.word; fields]),
            Some(SizeAlign {
                size: fields as u64 * 8,
                align: 8,
            }),
        );
        f.graph.oracle.record_semantic_aggregate(
            ty,
            SemanticAggregate {
                fields: vec![f.word; fields],
                variants: vec![],
            },
        );
        f.graph.oracle.record_aggregate(
            ty,
            AggregateLayout {
                fields: (0..fields)
                    .map(|i| FieldLayout {
                        ty: f.word,
                        offset: i as u64 * 8,
                    })
                    .collect(),
                variants: vec![],
            },
        );
        ty
    };
    let pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let field_ptr = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: f.word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let ValueType::Tuple(mut start) = f.graph.value_type(f.graph.start).clone() else {
        unreachable!()
    };
    start[2] = ValueType::Rust(pointer);
    f.graph
        .set_value_type(f.graph.start, ValueType::Tuple(start));
    f.graph.set_value_type(f.args[0], ValueType::Rust(pointer));
    let mut value = f.args[1];
    for field in 0..fields {
        let address = if fields == 1 {
            f.args[0]
        } else {
            f.node(Opcode::FieldAddr, &[f.args[0]], field as u32, field_ptr)
        };
        let load = f.graph.add_node(
            Opcode::LoadValue,
            f.control,
            &[f.effect, address],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(f.word),
            ]),
        );
        f.control = projection(&mut f.graph, load, 0, ValueType::Control);
        f.effect = projection(&mut f.graph, load, 1, ValueType::Effect);
        let part = projection(&mut f.graph, load, 2, ValueType::Rust(f.word));
        if !unused {
            value = f.node(Opcode::Add, &[value, part], 0, f.word);
        }
    }
    f.ret(value);
    f.verify();
    f
}

fn caller(source: &FunctionGraph, name: &str) -> FunctionGraph {
    let mut graph = source.clone();
    graph.symbol = name.into();
    let ids: Vec<_> = graph.ids().collect();
    for node in ids {
        if node != graph.start
            && !(graph.op(node) == Opcode::Projection && graph.input(node, 0) == graph.start)
        {
            graph.remove_node(node);
        }
    }
    let mut f = Function {
        control: graph.find_projection(graph.start, 0).unwrap(),
        effect: graph.find_projection(graph.start, 1).unwrap(),
        args: (0..2)
            .map(|i| graph.find_projection(graph.start, i + 2).unwrap())
            .collect(),
        word: source
            .rust_type(source.find_projection(source.start, 3).unwrap())
            .unwrap(),
        graph,
    };
    let args = f.args.clone();
    let value = f.call(source, &args);
    f.ret(value);
    f.verify();
    f.graph
}

struct Program<'a>(&'a BTreeMap<String, FunctionGraph>);
impl SemanticCallHandler for Program<'_> {
    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, InterpretError> {
        unreachable!()
    }
    fn call_with_memory(
        &mut self,
        symbol: &str,
        args: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        let graph = &self.0[symbol];
        match Interpreter::new(graph).run_values_in_memory(args, self, memory)? {
            SemanticOutcome::Return(value) => Ok(value),
            SemanticOutcome::Unreachable => {
                Err(InterpretError::UndefinedBehaviour("unreachable".into()))
            }
        }
    }
}

fn execute(
    graphs: &BTreeMap<String, FunctionGraph>,
    fields: usize,
    initialized: bool,
    poison: bool,
) -> Result<SemanticOutcome, InterpretError> {
    let mut memory = SemanticMemory::new();
    let address = memory.allocate(fields as u64 * 8, 8, true).unwrap();
    if initialized {
        let bytes: Vec<_> = (0..fields)
            .flat_map(|field| (field as u64 + 11).to_le_bytes())
            .map(Some)
            .collect();
        memory.write_bytes(address, &bytes).unwrap();
    }
    Interpreter::new(&graphs["caller"]).run_values_in_memory(
        &[
            Value::Scalar(address),
            if poison {
                Value::Poison
            } else {
                Value::Scalar(7)
            },
        ],
        &mut Program(graphs),
        &mut memory,
    )
}

#[test]
fn promotes_one_to_three_scalar_parts_and_updates_all_callers() {
    for fields in 1..=3 {
        let source = source(fields, false).graph;
        let mut graphs = BTreeMap::from_iter([
            (source.symbol.clone(), source.clone()),
            ("caller".into(), caller(&source, "caller")),
            ("other".into(), caller(&source, "other")),
        ]);
        let before = graphs.clone();
        let stats =
            promote_arguments(&mut graphs, &BTreeSet::from([source.symbol.clone()])).unwrap();
        assert_eq!(stats.arguments_promoted, 1);
        assert_eq!(stats.scalar_parts, fields);
        assert_eq!(stats.calls_changed, 2);
        for graph in graphs.values_mut() {
            verify_graph(graph).unwrap();
        }
        for initialized in [false, true] {
            for poison in [false, true] {
                let before = execute(&before, fields, initialized, poison);
                let after = execute(&graphs, fields, initialized, poison);
                match (&before, &after) {
                    (
                        Err(InterpretError::PoisonObserved(_)),
                        Err(InterpretError::PoisonObserved(_)),
                    ) => {}
                    _ => assert_eq!(before, after),
                }
            }
        }
    }
}

#[test]
fn unused_load_cannot_become_a_defined_scalar_argument() {
    let source = source(1, true).graph;
    let mut graphs = BTreeMap::from_iter([
        (source.symbol.clone(), source.clone()),
        ("caller".into(), caller(&source, "caller")),
    ]);
    assert_eq!(
        promote_arguments(&mut graphs, &BTreeSet::from([source.symbol]))
            .unwrap()
            .functions_changed,
        0
    );
    assert_eq!(
        execute(&graphs, 1, false, false),
        Ok(SemanticOutcome::Return(Value::Scalar(7)))
    );
}

#[test]
fn exported_addressed_and_large_aggregate_parameters_keep_their_abi() {
    for fields in [1, 4] {
        let source = source(fields, false).graph;
        let mut graphs = BTreeMap::from_iter([
            (source.symbol.clone(), source.clone()),
            ("caller".into(), caller(&source, "caller")),
        ]);
        assert_eq!(
            promote_arguments(&mut graphs, &BTreeSet::new())
                .unwrap()
                .functions_changed,
            0
        );
        let owner = graphs.get_mut("caller").unwrap();
        let payload = owner.symbols.len() as u32;
        owner.symbols.push((source.symbol.clone(), 0));
        let ty = source
            .value_type(source.find_projection(source.start, 2).unwrap())
            .clone();
        owner.add_node(Opcode::SymbolAddress, NodeId::NONE, &[], payload, ty);
        assert_eq!(
            promote_arguments(&mut graphs, &BTreeSet::from([source.symbol]))
                .unwrap()
                .functions_changed,
            0
        );
    }
}

#[test]
fn writes_and_unknown_calls_block_snapshot_promotion() {
    for call in [false, true] {
        let mut f = source(1, false);
        let ret = f
            .graph
            .ids()
            .find(|&n| f.graph.op(n) == Opcode::Return)
            .unwrap();
        f.control = f.graph.control(ret);
        f.effect = f.graph.input(ret, 0);
        let value = f.graph.input(ret, 1);
        f.graph.remove_node(ret);
        if call {
            let abi = f.graph.abi.clone();
            f.call_named("unknown", &abi, &f.args.clone());
        } else {
            let store = f.graph.add_node(
                Opcode::StoreValue,
                f.control,
                &[f.effect, f.args[0], f.args[1]],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
            );
            f.control = projection(&mut f.graph, store, 0, ValueType::Control);
            f.effect = projection(&mut f.graph, store, 1, ValueType::Effect);
        }
        f.ret(value);
        f.verify();
        let source = f.graph;
        let mut graphs = BTreeMap::from_iter([
            (source.symbol.clone(), source.clone()),
            ("caller".into(), caller(&source, "caller")),
        ]);
        assert_eq!(
            promote_arguments(&mut graphs, &BTreeSet::from([source.symbol]))
                .unwrap()
                .functions_changed,
            0
        );
    }
}

#[test]
fn reference_place_casts_and_sequential_observations_allow_promotion() {
    let mut f = source(2, false);
    let pointer = f.graph.rust_type(f.args[0]).unwrap();
    let TypeShape::RawPointer { pointee, .. } = *f.graph.oracle.table.shape(pointer) else {
        unreachable!()
    };
    let reference = f.graph.oracle.intern(
        TypeShape::Reference {
            mutability: Mutability::Not,
            pointee,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let place = f.graph.oracle.intern(
        TypeShape::PlaceAddress { pointee },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let ValueType::Tuple(mut start) = f.graph.value_type(f.graph.start).clone() else {
        unreachable!()
    };
    start[2] = ValueType::Rust(reference);
    f.graph
        .set_value_type(f.graph.start, ValueType::Tuple(start));
    f.graph
        .set_value_type(f.args[0], ValueType::Rust(reference));
    let uses: Vec<_> = f
        .graph
        .ids()
        .filter(|&node| {
            f.graph.op(node) == Opcode::FieldAddr && f.graph.input(node, 0) == f.args[0]
        })
        .collect();
    for node in uses {
        let cast = f.node(Opcode::PointerCast, &[f.args[0]], 0, place);
        f.graph.set_input(node, 0, cast);
    }
    let ret = f
        .graph
        .ids()
        .find(|&node| f.graph.op(node) == Opcode::Return)
        .unwrap();
    let result = f.graph.input(ret, 1);
    f.control = f.graph.control(ret);
    f.effect = f.graph.input(ret, 0);
    f.graph.remove_node(ret);
    let loaded: Vec<_> = f
        .graph
        .ids()
        .filter(|&node| f.graph.op(node) == Opcode::LoadValue)
        .map(|node| f.graph.find_projection(node, 2).unwrap())
        .collect();
    for value in loaded {
        f.observe(value);
    }
    f.ret(result);
    f.verify();
    let source = f.graph;
    let mut graphs = BTreeMap::from_iter([
        (source.symbol.clone(), source.clone()),
        ("caller".into(), caller(&source, "caller")),
    ]);
    let before = graphs.clone();
    let stats = promote_arguments(&mut graphs, &BTreeSet::from([source.symbol])).unwrap();
    assert_eq!(stats.scalar_parts, 2);
    for graph in graphs.values_mut() {
        verify_graph(graph).unwrap();
    }
    for initialized in [false, true] {
        let before = execute(&before, 2, initialized, false);
        let after = execute(&graphs, 2, initialized, false);
        match (&before, &after) {
            (Err(InterpretError::PoisonObserved(_)), Err(InterpretError::PoisonObserved(_))) => {}
            _ => assert_eq!(before, after),
        }
    }
}
