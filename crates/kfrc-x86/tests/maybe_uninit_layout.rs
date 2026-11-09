//! Opaque snapshots preserve bytes and parallel Phi semantics across S→L.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::lower_function;

fn local(body: &mut BodyBuilder, ty: TyId) -> LocalId {
    body.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}
fn copy(local: LocalId) -> Operand {
    Operand::Copy(Place::local(local))
}
fn assign(body: &mut BodyBuilder, target: LocalId, value: Rvalue) {
    body.push_statement(StatementKind::Assign(Place::local(target), value));
}
fn constant(body: &mut BodyBuilder, ty: TyId, value: u128) -> Operand {
    Operand::Constant(body.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(value),
    }))
}
fn mu(types: &mut RecordedTypes, inner: TyId, size: u64, align: u64) -> TyId {
    let ty = types.intern(
        TypeShape::Adt {
            name: format!("MaybeUninit{}", inner.0),
        },
        Some(SizeAlign { size, align }),
    );
    types.record_maybe_uninit(ty, inner);
    ty
}

fn swap_loop() -> Body {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let opaque = mu(&mut types, word, 8, 8);
    let mut b = BodyBuilder::new(
        "opaque_swap",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut b, word);
    let n = local(&mut b, word);
    b.set_arg_count(1);
    let x = local(&mut b, opaque);
    let y = local(&mut b, opaque);
    let temp = local(&mut b, opaque);
    let index = local(&mut b, word);
    let condition = local(&mut b, boolean);
    let a = constant(&mut b, word, 11);
    let z = constant(&mut b, word, 22);
    let zero = constant(&mut b, word, 0);
    assign(&mut b, x, Rvalue::Cast(CastKind::Transmute, a, opaque));
    assign(&mut b, y, Rvalue::Cast(CastKind::Transmute, z, opaque));
    assign(&mut b, index, Rvalue::Use(zero));
    b.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(
        &mut b,
        condition,
        Rvalue::BinaryOp(BinaryOp::Lt, copy(index), copy(n)),
    );
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(condition),
            targets: SwitchTargets {
                cases: vec![(1, BlockId(2))],
                otherwise: BlockId(3),
            },
        },
        false,
    );
    assign(&mut b, temp, Rvalue::Use(copy(x)));
    assign(&mut b, x, Rvalue::Use(copy(y)));
    assign(&mut b, y, Rvalue::Use(copy(temp)));
    let one = constant(&mut b, word, 1);
    assign(
        &mut b,
        index,
        Rvalue::BinaryOp(BinaryOp::Add, copy(index), one),
    );
    b.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(
        &mut b,
        ret,
        Rvalue::Cast(CastKind::Transmute, copy(x), word),
    );
    b.finish_block(Terminator::Return, false);
    b.finish()
}

fn opaque_bool() -> Body {
    let mut types = RecordedTypes::new();
    let byte = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let opaque = mu(&mut types, boolean, 1, 1);
    let mut body = BodyBuilder::new(
        "opaque_bool",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut body, byte);
    let input = local(&mut body, byte);
    body.set_arg_count(1);
    let storage = local(&mut body, opaque);
    assign(
        &mut body,
        storage,
        Rvalue::Cast(CastKind::Transmute, copy(input), opaque),
    );
    assign(
        &mut body,
        ret,
        Rvalue::Cast(CastKind::Transmute, copy(storage), byte),
    );
    body.finish_block(Terminator::Return, false);
    body.finish()
}

fn compile(body: &Body) -> FunctionGraph {
    let mut graph = build_graph(body).unwrap();
    verify_graph(&mut graph).unwrap_or_else(|e| panic!("{e}\n{}", graph.dump()));
    lower_layout(&mut graph).unwrap_or_else(|e| panic!("{e}\n{}", graph.dump()));
    verify_graph(&mut graph).unwrap_or_else(|e| panic!("{e}\n{}", graph.dump()));
    lower_function(&graph).unwrap();
    graph
}

fn partial_snapshot(size: u64, read_uninitialized: bool) -> FunctionGraph {
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
            width: IntegerWidth::Pointer,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let array = types.intern(
        TypeShape::Array {
            element: byte,
            length: size,
        },
        Some(SizeAlign { size, align: 1 }),
    );
    let opaque = mu(&mut types, array, size, 1);
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: opaque,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let byte_pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: byte,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut graph = FunctionGraph::new(
        "partial_snapshot",
        FunctionAbi {
            args: vec![],
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
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    graph.start = start;
    let mut control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let mut effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let source = graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    let destination = graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        1,
        ValueType::Rust(pointer),
    );
    let source_byte = graph.add_node(
        Opcode::PointerCast,
        NodeId::NONE,
        &[source],
        0,
        ValueType::Rust(byte_pointer),
    );
    fn fixed(
        graph: &mut FunctionGraph,
        control: &mut NodeId,
        effect: &mut NodeId,
        opcode: Opcode,
        args: &[NodeId],
        result: Option<TyId>,
    ) -> Option<NodeId> {
        let mut inputs = vec![*effect];
        inputs.extend_from_slice(args);
        let mut results = vec![ValueType::Control, ValueType::Effect];
        if let Some(ty) = result {
            results.push(ValueType::Rust(ty));
        }
        let node = graph.add_node(opcode, *control, &inputs, 0, ValueType::Tuple(results));
        *control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            0,
            ValueType::Control,
        );
        *effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            1,
            ValueType::Effect,
        );
        result.map(|ty| {
            graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[node],
                2,
                ValueType::Rust(ty),
            )
        })
    }
    let old = graph.add_constant(37, byte);
    fixed(
        &mut graph,
        &mut control,
        &mut effect,
        Opcode::StoreValue,
        &[source_byte, old],
        None,
    );
    let snapshot = fixed(
        &mut graph,
        &mut control,
        &mut effect,
        Opcode::LoadValue,
        &[source],
        Some(opaque),
    )
    .unwrap();
    let new = graph.add_constant(99, byte);
    fixed(
        &mut graph,
        &mut control,
        &mut effect,
        Opcode::StoreValue,
        &[source_byte, new],
        None,
    );
    fixed(
        &mut graph,
        &mut control,
        &mut effect,
        Opcode::StoreValue,
        &[destination, snapshot],
        None,
    );
    let destination_byte = graph.add_node(
        Opcode::PointerCast,
        NodeId::NONE,
        &[destination],
        0,
        ValueType::Rust(byte_pointer),
    );
    let index = graph.add_constant(u128::from(read_uninitialized), word);
    let address = graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[destination_byte, index],
        0,
        ValueType::Rust(byte_pointer),
    );
    let value = fixed(
        &mut graph,
        &mut control,
        &mut effect,
        Opcode::LoadValue,
        &[address],
        Some(byte),
    )
    .unwrap();
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    graph
}

#[test]
fn opaque_copy_keeps_partial_initialization_and_owned_snapshots_without_byte_expansion() {
    let mut sizes = Vec::new();
    for size in [16, 4096] {
        for read_uninitialized in [false, true] {
            let mut graph = partial_snapshot(size, read_uninitialized);
            verify_graph(&mut graph).unwrap();
            let semantic = Interpreter::new(&graph)
                .run_values(&[], &mut |_: &str, _: &[Value]| Ok(Value::Scalar(0)));
            lower_layout(&mut graph).unwrap();
            verify_graph(&mut graph).unwrap_or_else(|e| panic!("{e}\n{}", graph.dump()));
            let physical = Interpreter::new(&graph).run(&[], &mut |_: &str, _: &[u64]| Ok(0));
            if read_uninitialized {
                assert!(matches!(semantic, Err(InterpretError::PoisonObserved(_))));
                assert!(matches!(physical, Err(InterpretError::PoisonObserved(_))));
            } else {
                assert_eq!(
                    semantic.unwrap(),
                    SemanticOutcome::Return(Value::Scalar(37))
                );
                assert_eq!(physical.unwrap(), Outcome::Return(37));
                sizes.push(graph.len());
            }
            assert_eq!(
                graph.ids().filter(|&n| graph.op(n) == Opcode::Load).count(),
                1
            );
        }
    }
    assert_eq!(sizes[0], sizes[1]);
}

#[test]
fn opaque_phi_swap_has_parallel_copy_semantics() {
    let body = swap_loop();
    let mut semantic = build_graph(&body).unwrap();
    verify_graph(&mut semantic).unwrap();
    let layout = compile(&body);
    for count in 0..20 {
        let expected = if count % 2 == 0 { 11 } else { 22 };
        let source = Interpreter::new(&semantic)
            .run_values(&[Value::Scalar(count)], &mut |_: &str, _: &[Value]| {
                Ok(Value::Scalar(0))
            })
            .unwrap();
        let target = Interpreter::new(&layout)
            .run(&[count], &mut |_: &str, _: &[u64]| Ok(0))
            .unwrap();
        assert_eq!(source, SemanticOutcome::Return(Value::Scalar(expected)));
        assert_eq!(target, Outcome::Return(expected));
    }
}

#[test]
fn invalid_inner_bool_is_only_opaque_bytes_until_typed_observation() {
    let graph = compile(&opaque_bool());
    for value in 0..=255 {
        assert_eq!(
            Interpreter::new(&graph)
                .run(&[value], &mut |_: &str, _: &[u64]| Ok(0))
                .unwrap(),
            Outcome::Return(value)
        );
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn encoded_opaque_snapshots_and_phi_cycles_match_c() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::process::Command;
    let mut functions = [swap_loop(), opaque_bool()]
        .into_iter()
        .map(|body| FunctionCode {
            function: lower_function(&compile(&body)).unwrap(),
            binding: SymbolBinding::Global { hidden: false },
        })
        .collect::<Vec<_>>();
    let mut partial = partial_snapshot(4096, false);
    verify_graph(&mut partial).unwrap();
    lower_layout(&mut partial).unwrap();
    verify_graph(&mut partial).unwrap();
    functions.push(FunctionCode {
        function: lower_function(&partial).unwrap(),
        binding: SymbolBinding::Global { hidden: false },
    });
    let directory = std::env::temp_dir().join(format!("kfrc-maybe-uninit-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let object = directory.join("opaque.o");
    let source = directory.join("main.c");
    let executable = directory.join("check");
    std::fs::write(&object, write_object(&functions).unwrap()).unwrap();
    std::fs::write(
        &source,
        r#"
#include <stdint.h>
extern uint64_t opaque_swap(uint64_t);
extern uint8_t opaque_bool(uint8_t);
extern uint8_t partial_snapshot(void);
int main(void) {
    if (partial_snapshot() != 37) return 3;
    for (unsigned i=0;i<256;i++) {
        if (opaque_swap(i) != (i%2 ? 22 : 11)) return 1;
        if (opaque_bool(i) != i) return 2;
    }
    return 0;
}
"#,
    )
    .unwrap();
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
